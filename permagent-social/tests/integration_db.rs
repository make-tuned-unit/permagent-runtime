use permagent_social::db;
use tempfile::TempDir;

#[tokio::test]
async fn test_migrate_and_seed() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.db");

    let pool = db::connect(&db_path).await.expect("connect");
    db::migrate(&pool).await.expect("migrate");
    db::seed_default_projects(&pool).await.expect("seed");

    // Verify three projects exist
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(&pool)
        .await
        .expect("count projects");
    assert_eq!(count, 3);

    // Verify each has 4 social columns + 4 coding columns = 8
    let col_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_columns")
        .fetch_one(&pool)
        .await
        .expect("count columns");
    assert_eq!(col_count, 24); // 3 projects × 8 columns

    // Verify Atlas Atlantic has correct social columns in correct order
    let cols: Vec<(String, i64)> = sqlx::query_as(
        "SELECT name, position FROM board_columns
         WHERE project_id = (SELECT id FROM projects WHERE slug = 'atlas-atlantic')
         AND card_type = 'social_post'
         ORDER BY position"
    )
    .fetch_all(&pool)
    .await
    .expect("fetch columns");

    assert_eq!(cols.len(), 4);
    assert_eq!(cols[0], ("Draft".to_string(), 0));
    assert_eq!(cols[1], ("Scheduled".to_string(), 1));
    assert_eq!(cols[2], ("Posted".to_string(), 2));
    assert_eq!(cols[3], ("Failed".to_string(), 3));
}

#[tokio::test]
async fn test_seed_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.db");

    let pool = db::connect(&db_path).await.unwrap();
    db::migrate(&pool).await.unwrap();
    db::seed_default_projects(&pool).await.unwrap();
    db::seed_default_projects(&pool).await.unwrap(); // run twice

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3, "seeding should be idempotent");
}
