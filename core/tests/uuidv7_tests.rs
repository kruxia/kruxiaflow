//! The uuidv7() used for primary key defaults: native on PostgreSQL 18+, a
//! PL/pgSQL implementation (created by migration) before that.

use sqlx::PgPool;
use uuid::Uuid;

/// Regression: the PostgreSQL < 18 implementation keeps its state in session
/// variables, and a connection whose first uuidv7() call was rolled back
/// failed every later call with `invalid input syntax for type bigint: ""`.
#[sqlx::test(migrations = "../migrations")]
async fn test_uuidv7_works_after_rolled_back_first_call(pool: PgPool) {
    let mut conn = pool.acquire().await.unwrap();

    sqlx::query("BEGIN").execute(&mut *conn).await.unwrap();
    let _: Uuid = sqlx::query_scalar("SELECT uuidv7()")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    sqlx::query("ROLLBACK").execute(&mut *conn).await.unwrap();

    let first: Uuid = sqlx::query_scalar("SELECT uuidv7()")
        .fetch_one(&mut *conn)
        .await
        .expect("uuidv7() after a rolled-back first call");
    let second: Uuid = sqlx::query_scalar("SELECT uuidv7()")
        .fetch_one(&mut *conn)
        .await
        .unwrap();

    assert_eq!(first.get_version_num(), 7);
    assert!(
        second > first,
        "uuidv7() must stay monotonic on a connection"
    );
}

/// Ids from one connection are strictly increasing, including within a
/// millisecond
#[sqlx::test(migrations = "../migrations")]
async fn test_uuidv7_is_monotonic_on_a_connection(pool: PgPool) {
    let mut conn = pool.acquire().await.unwrap();
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT uuidv7() FROM generate_series(1, 500)")
        .fetch_all(&mut *conn)
        .await
        .unwrap();

    assert_eq!(ids.len(), 500);
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
}
