use paperless_llm::{
    AppError,
    budget::{BudgetLimits, BudgetService, ReservationContext},
    db::Database,
};
use tempfile::TempDir;

async fn fixture() -> (TempDir, Database) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = Database::connect(&directory.path().join("test.db"))
        .await
        .expect("database");
    sqlx::query(
        "INSERT INTO paperless_connections(id,name,base_url,token_secret_key) VALUES('paperless','Paperless','http://localhost:8000','token')",
    )
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles(id,name,provider_kind,base_url,model,api_family) VALUES('provider','Local','lmstudio','http://localhost:1234/v1','model','responses')",
    )
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO processing_profiles(id,name,paperless_connection_id,provider_profile_id,enabled) VALUES('profile','Default','paperless','provider',1)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    for id in ["job-a", "job-b"] {
        sqlx::query(
            "INSERT INTO jobs(id,processing_profile_id,profile_revision,paperless_document_id,document_checksum,status) VALUES(?, 'profile', 1, ?, ?, 'queued')",
        )
        .bind(id)
        .bind(if id == "job-a" { 1_i64 } else { 2_i64 })
        .bind(format!("checksum-{id}"))
        .execute(database.pool())
        .await
        .unwrap();
    }
    (directory, database)
}

#[tokio::test]
async fn migration_has_no_foreign_key_violations() {
    let (_directory, database) = fixture().await;
    let violations: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(database.pool())
        .await
        .unwrap();
    assert!(violations.is_empty());
}

#[tokio::test]
async fn live_polling_queries_have_dedicated_indexes() {
    let (_directory, database) = fixture().await;
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='index' AND name IN (\
            'idx_jobs_updated_at','idx_jobs_status_updated_at','idx_usage_events_job_id'\
         ) ORDER BY name",
    )
    .fetch_all(database.pool())
    .await
    .unwrap();
    assert_eq!(
        indexes,
        vec![
            "idx_jobs_status_updated_at".to_owned(),
            "idx_jobs_updated_at".to_owned(),
            "idx_usage_events_job_id".to_owned(),
        ]
    );

    let job_plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN \
         SELECT id FROM jobs WHERE status='queued' ORDER BY updated_at DESC LIMIT 50",
    )
    .fetch_all(database.pool())
    .await
    .unwrap();
    assert!(
        job_plan
            .iter()
            .any(|(_, _, _, detail)| detail.contains("idx_jobs_status_updated_at"))
    );

    let usage_plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT SUM(total_tokens) FROM usage_events WHERE job_id='job-a'",
    )
    .fetch_all(database.pool())
    .await
    .unwrap();
    assert!(
        usage_plan
            .iter()
            .any(|(_, _, _, detail)| detail.contains("idx_usage_events_job_id"))
    );
}

#[tokio::test]
async fn competing_budget_reservations_cannot_oversubscribe() {
    let (_directory, database) = fixture().await;
    let budget = BudgetService::new(database);
    let limits = BudgetLimits {
        global_daily: Some(100),
        global_monthly: Some(100),
        provider_daily: None,
        provider_monthly: None,
        timezone: "Europe/Berlin".to_owned(),
    };
    let first = budget.reserve(Some("job-a"), "provider", 60, &limits);
    let second = budget.reserve(Some("job-b"), "provider", 60, &limits);
    let (first, second) = tokio::join!(first, second);
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let error = first
        .err()
        .or_else(|| second.err())
        .expect("one blocked call");
    assert!(matches!(error, AppError::Budget(_)));
}

#[tokio::test]
async fn startup_recovery_full_charges_orphan_reservation_into_usage_history() {
    let (_directory, database) = fixture().await;
    let budget = BudgetService::new(database.clone());
    let reservation = budget
        .reserve_attempt(
            None,
            "provider",
            80,
            &BudgetLimits {
                timezone: "Europe/Berlin".to_owned(),
                ..BudgetLimits::default()
            },
            &ReservationContext {
                operation: "provider_test".to_owned(),
                api_family: "responses".to_owned(),
                functions_json: "[]".to_owned(),
                input_tokens: 30,
            },
        )
        .await
        .unwrap();

    assert_eq!(budget.reconcile_stale_reservations().await.unwrap(), 1);
    assert_eq!(budget.reconcile_stale_reservations().await.unwrap(), 0);
    let state: String = sqlx::query_scalar("SELECT state FROM budget_reservations WHERE id=?")
        .bind(&reservation.id)
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(state, "charged_unknown");
    let usage: (Option<String>, String, i64, i64, i64, String, String) = sqlx::query_as(
        "SELECT job_id,operation,input_tokens,output_tokens,total_tokens,usage_source,error_code FROM usage_events WHERE budget_reservation_id=?",
    )
    .bind(&reservation.id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(usage.0, None);
    assert_eq!(usage.1, "provider_test");
    assert_eq!((usage.2, usage.3, usage.4), (30, 50, 80));
    assert_eq!(usage.5, "reserved_after_unknown");
    assert_eq!(usage.6, "interrupted_unknown");
}
