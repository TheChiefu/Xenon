use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::models::{GlobalRole, UserSummary};

/// One page of users
/// - pool: Pool of SQL Connections
/// - match_user: Username to match query on
/// - after: User id to page from ('None' shows first 'limit' amount of results)
/// - limit: How many users to return
pub async fn get_users(
    pool: &sqlx::SqlitePool,
    match_user: Option<String>,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<UserSummary>> {

    let mut conn = pool.acquire().await?;

    // Escape patterns for SQL string
    let pattern = match_user.map(|name| {
        let escaped = name
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("{escaped}%")
    });

    let users: Vec<UserSummary> = sqlx::query_as(
        "
        SELECT id, username, display_name
        FROM users u
        WHERE (?1 IS NULL OR u.id > ?1)
            AND (?2 IS NULL OR u.username LIKE ?2 ESCAPE '\\')
            AND u.deleted_at IS NULL
        ORDER BY id
        LIMIT ?3
        "
    )
    .bind(after)
    .bind(pattern)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;

    Ok(users)

}

/// Looks up a user's public profile by id.
///
/// Not filtered by deleted_at: a soft-deleted user's past messages still need
/// a name to display, so their profile stays resolvable after they're gone.
/// - pool: Pool of SQL Connections
/// - user_id: User to look up
pub async fn get_user(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
) -> Result<UserSummary> {

    let mut conn = pool.acquire().await?;

    let user: Option<UserSummary> = sqlx::query_as(
        "
        SELECT id, username, display_name
        FROM users
        WHERE id = ?1
        "
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;

    user.ok_or(AppError::NotFound)
}

/// Changes a user's global role
/// - pool: Pool of SQL connections
/// - actor_id:  User performing the role change
/// - target_id: User receiving the role change
/// - role: The global role target receives
pub async fn set_role(
    pool: &sqlx::SqlitePool,
    actor_id: Uuid,
    target_id: Uuid,
    role: GlobalRole,
) -> Result<()> {

    // Can't change owner's role
    if role == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    // Start transaction
    let mut tx = pool.begin().await?;

    let actor_role = db::global_role(&mut *tx, actor_id).await?;

    // If actor a rank high enough to change other's roles
    let permissiable_roles = [GlobalRole::Owner, GlobalRole::Admin];
    if !permissiable_roles.contains(&actor_role) {
        return Err(AppError::Forbidden);
    }

    // Can't change owner's role
    let target_role = db::global_role(&mut *tx, target_id).await?;
    if target_role == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    // Admins can't change each other's roles
    if actor_role == GlobalRole::Admin && target_role == GlobalRole::Admin {
        return Err(AppError::Forbidden);
    }

    // Rank doesn't exist, exit
    if !db::set_global_role(&mut *tx, target_id, role).await? {
        return Err(AppError::NotFound);
    }

    tx.commit().await?;
    Ok(())

}
