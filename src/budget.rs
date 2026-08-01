use chrono::{DateTime, Datelike, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use uuid::Uuid;

use crate::{AppError, AppResult, db::Database};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetLimits {
    pub global_daily: Option<i64>,
    pub global_monthly: Option<i64>,
    pub provider_daily: Option<i64>,
    pub provider_monthly: Option<i64>,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reservation {
    pub id: String,
    pub reserved_tokens: i64,
    pub daily_used_before: i64,
    pub monthly_used_before: i64,
}

#[derive(Debug, Clone)]
pub struct ReservationContext {
    pub operation: String,
    pub api_family: String,
    pub functions_json: String,
    pub input_tokens: i64,
}

impl Default for ReservationContext {
    fn default() -> Self {
        Self {
            operation: "unknown".to_owned(),
            api_family: "unknown".to_owned(),
            functions_json: "[]".to_owned(),
            input_tokens: 0,
        }
    }
}

#[derive(Clone)]
pub struct BudgetService {
    database: Database,
}

impl BudgetService {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn reserve(
        &self,
        job_id: Option<&str>,
        provider_profile_id: &str,
        requested_tokens: i64,
        limits: &BudgetLimits,
    ) -> AppResult<Reservation> {
        self.reserve_attempt(
            job_id,
            provider_profile_id,
            requested_tokens,
            limits,
            &ReservationContext::default(),
        )
        .await
    }

    pub async fn reserve_attempt(
        &self,
        job_id: Option<&str>,
        provider_profile_id: &str,
        requested_tokens: i64,
        limits: &BudgetLimits,
        context: &ReservationContext,
    ) -> AppResult<Reservation> {
        if requested_tokens <= 0 {
            return Err(AppError::Validation(
                "budget reservation must be positive".to_owned(),
            ));
        }
        let timezone: Tz = limits.timezone.parse().map_err(|_| {
            AppError::Validation(format!("unknown budget timezone: {}", limits.timezone))
        })?;
        let now = Utc::now();
        let periods = BudgetPeriods {
            daily_start: period_start(now, timezone, false)?,
            monthly_start: period_start(now, timezone, true)?,
        };

        let mut connection = self.database.pool().acquire().await?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await?;
        let result = reserve_in_transaction(
            &mut connection,
            job_id,
            provider_profile_id,
            requested_tokens,
            limits,
            context,
            &periods,
        )
        .await;
        match result {
            Ok(reservation) => {
                sqlx::query("COMMIT").execute(&mut *connection).await?;
                Ok(reservation)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                Err(error)
            }
        }
    }

    pub async fn settle(&self, reservation_id: &str, actual_tokens: i64) -> AppResult<()> {
        if actual_tokens < 0 {
            return Err(AppError::Validation(
                "actual token usage cannot be negative".to_owned(),
            ));
        }
        let result = sqlx::query(
            "UPDATE budget_reservations SET state='settled', settled_tokens=?, settled_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') \
             WHERE id=? AND state='reserved'",
        )
        .bind(actual_tokens)
        .bind(reservation_id)
        .execute(self.database.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::Conflict(
                "budget reservation is no longer active".to_owned(),
            ));
        }
        Ok(())
    }

    pub async fn charge_unknown(&self, reservation_id: &str) -> AppResult<i64> {
        let reserved: Option<i64> = sqlx::query_scalar(
            "SELECT reserved_tokens FROM budget_reservations WHERE id=? AND state='reserved'",
        )
        .bind(reservation_id)
        .fetch_optional(self.database.pool())
        .await?;
        let reserved = reserved.ok_or_else(|| {
            AppError::Conflict("budget reservation is no longer active".to_owned())
        })?;
        sqlx::query(
            "UPDATE budget_reservations SET state='charged_unknown', settled_tokens=reserved_tokens, settled_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
        )
        .bind(reservation_id)
        .execute(self.database.pool())
        .await?;
        Ok(reserved)
    }

    pub async fn release(&self, reservation_id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE budget_reservations SET state='released', settled_tokens=0, settled_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND state='reserved'",
        )
        .bind(reservation_id)
        .execute(self.database.pool())
        .await?;
        Ok(())
    }

    /// Every reservation found during process startup belongs to an inference
    /// whose outcome cannot be known after the process that dispatched it was
    /// interrupted. Charge the full reservation and materialize a usage event
    /// so budgets and lifetime statistics remain consistent.
    pub async fn reconcile_stale_reservations(&self) -> AppResult<u64> {
        let mut connection = self.database.pool().acquire().await?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await?;
        let result = reconcile_stale_in_transaction(&mut connection).await;
        match result {
            Ok(reconciled) => {
                sqlx::query("COMMIT").execute(&mut *connection).await?;
                Ok(reconciled)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                Err(error)
            }
        }
    }
}

struct BudgetPeriods {
    daily_start: DateTime<Utc>,
    monthly_start: DateTime<Utc>,
}

async fn reserve_in_transaction(
    connection: &mut SqliteConnection,
    job_id: Option<&str>,
    provider_profile_id: &str,
    requested_tokens: i64,
    limits: &BudgetLimits,
    context: &ReservationContext,
    periods: &BudgetPeriods,
) -> AppResult<Reservation> {
    let provider_daily =
        used_tokens(connection, Some(provider_profile_id), periods.daily_start).await?;
    let provider_monthly =
        used_tokens(connection, Some(provider_profile_id), periods.monthly_start).await?;
    let global_daily = used_tokens(connection, None, periods.daily_start).await?;
    let global_monthly = used_tokens(connection, None, periods.monthly_start).await?;

    check_limit(
        "global daily",
        global_daily,
        requested_tokens,
        limits.global_daily,
    )?;
    check_limit(
        "global monthly",
        global_monthly,
        requested_tokens,
        limits.global_monthly,
    )?;
    check_limit(
        "provider daily",
        provider_daily,
        requested_tokens,
        limits.provider_daily,
    )?;
    check_limit(
        "provider monthly",
        provider_monthly,
        requested_tokens,
        limits.provider_monthly,
    )?;

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO budget_reservations(id, job_id, provider_profile_id, reserved_tokens, state, operation, api_family, functions_json, input_tokens) VALUES(?, ?, ?, ?, 'reserved', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(job_id)
    .bind(provider_profile_id)
    .bind(requested_tokens)
    .bind(&context.operation)
    .bind(&context.api_family)
    .bind(&context.functions_json)
    .bind(context.input_tokens.clamp(0, requested_tokens))
    .execute(&mut *connection)
    .await?;
    Ok(Reservation {
        id,
        reserved_tokens: requested_tokens,
        daily_used_before: global_daily,
        monthly_used_before: global_monthly,
    })
}

#[derive(sqlx::FromRow)]
struct StaleReservation {
    id: String,
    job_id: Option<String>,
    provider_profile_id: String,
    reserved_tokens: i64,
    operation: String,
    api_family: String,
    functions_json: String,
    input_tokens: i64,
    provider_kind: String,
    model: String,
}

async fn reconcile_stale_in_transaction(connection: &mut SqliteConnection) -> AppResult<u64> {
    let reservations: Vec<StaleReservation> = sqlx::query_as(
        "SELECT r.id,r.job_id,r.provider_profile_id,r.reserved_tokens,r.operation,COALESCE(r.api_family,p.api_family) AS api_family,r.functions_json,r.input_tokens,p.provider_kind,p.model \
         FROM budget_reservations r JOIN provider_profiles p ON p.id=r.provider_profile_id \
         WHERE r.state='reserved' ORDER BY r.created_at,r.id",
    )
    .fetch_all(&mut *connection)
    .await?;
    let mut reconciled = 0_u64;
    for reservation in reservations {
        let changed = sqlx::query(
            "UPDATE budget_reservations SET state='charged_unknown', settled_tokens=reserved_tokens, settled_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND state='reserved'",
        )
        .bind(&reservation.id)
        .execute(&mut *connection)
        .await?;
        if changed.rows_affected() != 1 {
            continue;
        }
        let input = reservation
            .input_tokens
            .clamp(0, reservation.reserved_tokens);
        let output = reservation.reserved_tokens.saturating_sub(input);
        sqlx::query(
            "INSERT INTO usage_events(id,budget_reservation_id,job_id,provider_profile_id,provider_kind,model,api_family,operation,functions_json,status,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,reasoning_tokens,total_tokens,usage_source,latency_ms,error_code) \
             VALUES(?,?,?,?,?,?,?,?,?,'failed',?,0,0,?,0,?,'reserved_after_unknown',0,'interrupted_unknown')",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&reservation.id)
        .bind(&reservation.job_id)
        .bind(&reservation.provider_profile_id)
        .bind(&reservation.provider_kind)
        .bind(&reservation.model)
        .bind(&reservation.api_family)
        .bind(&reservation.operation)
        .bind(&reservation.functions_json)
        .bind(input)
        .bind(output)
        .bind(reservation.reserved_tokens)
        .execute(&mut *connection)
        .await?;
        reconciled = reconciled.saturating_add(1);
    }
    Ok(reconciled)
}

async fn used_tokens(
    connection: &mut SqliteConnection,
    provider_profile_id: Option<&str>,
    start: DateTime<Utc>,
) -> AppResult<i64> {
    let start = start.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let usage: i64 = if let Some(provider_id) = provider_profile_id {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(total_tokens),0) FROM usage_events WHERE provider_profile_id=? AND created_at>=?",
        )
        .bind(provider_id)
        .bind(&start)
        .fetch_one(&mut *connection)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(total_tokens),0) FROM usage_events WHERE created_at>=?",
        )
        .bind(&start)
        .fetch_one(&mut *connection)
        .await?
    };
    let reserved: i64 = if let Some(provider_id) = provider_profile_id {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(reserved_tokens),0) FROM budget_reservations WHERE provider_profile_id=? AND state='reserved' AND created_at>=?",
        )
        .bind(provider_id)
        .bind(&start)
        .fetch_one(&mut *connection)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(reserved_tokens),0) FROM budget_reservations WHERE state='reserved' AND created_at>=?",
        )
        .bind(&start)
        .fetch_one(&mut *connection)
        .await?
    };
    Ok(usage + reserved)
}

fn check_limit(name: &str, used: i64, requested: i64, limit: Option<i64>) -> AppResult<()> {
    if let Some(limit) = limit.filter(|limit| *limit > 0)
        && used.saturating_add(requested) > limit
    {
        return Err(AppError::Budget(format!(
            "{name} token budget has {remaining} tokens remaining but this request reserves {requested}",
            remaining = limit.saturating_sub(used)
        )));
    }
    Ok(())
}

pub(crate) fn period_start(
    now: DateTime<Utc>,
    timezone: Tz,
    monthly: bool,
) -> AppResult<DateTime<Utc>> {
    let local = now.with_timezone(&timezone);
    let date = if monthly {
        NaiveDate::from_ymd_opt(local.year(), local.month(), 1)
    } else {
        Some(local.date_naive())
    }
    .ok_or_else(|| AppError::Internal("could not calculate budget period".to_owned()))?;
    let naive = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::Internal("could not calculate budget period".to_owned()))?;
    let local_start = match timezone.from_local_datetime(&naive) {
        LocalResult::Single(value) => value,
        LocalResult::Ambiguous(first, second) => first.min(second),
        LocalResult::None => timezone
            .from_local_datetime(&(naive + chrono::TimeDelta::hours(1)))
            .earliest()
            .ok_or_else(|| {
                AppError::Internal("budget timezone has no valid period start".to_owned())
            })?,
    };
    Ok(local_start.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn berlin_daily_start_handles_dst() {
        let now = DateTime::parse_from_rfc3339("2026-08-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let start = period_start(now, chrono_tz::Europe::Berlin, false).unwrap();
        assert_eq!(start.to_rfc3339(), "2026-07-31T22:00:00+00:00");
    }

    #[test]
    fn limit_is_checked_before_dispatch() {
        assert!(check_limit("daily", 900, 101, Some(1000)).is_err());
        assert!(check_limit("daily", 900, 100, Some(1000)).is_ok());
        assert!(check_limit("daily", 900, 1000, None).is_ok());
    }
}
