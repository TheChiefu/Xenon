//! The `users` table.

use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::models::{GlobalRole, UserSummary};

// API Methods //

/// Lists one page of users, optionally filtered by a username prefix.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `match_user` - Username prefix to match on, or `None` for every user.
/// * `after` - User id to page from, or `None` for the first page.
/// * `limit` - How many users to return.
pub async fn list(
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
        SELECT id, username, display_name, global_role
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

/// Reads a user's public profile by id.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - User to look up.
///
/// # Errors
///
/// Returns `AppError::NotFound` if no such user exists.
pub async fn get(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
) -> Result<UserSummary> {

    let mut conn = pool.acquire().await?;

    let user: Option<UserSummary> = sqlx::query_as(
        "
        SELECT id, username, display_name, global_role
        FROM users
        WHERE id = ?1
        "
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;

    user.ok_or(AppError::NotFound)
}

/// Changes a user's server-wide role.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `caller_id` - User performing the role change.
/// * `target_id` - User receiving the role change.
/// * `role` - Role the target receives.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the new role is Owner, if the caller is
/// neither Owner nor Admin, if the target is the Owner, or if an Admin targets
/// another Admin; and `AppError::NotFound` if the target does not exist.
pub async fn set_role(
    pool: &sqlx::SqlitePool,
    caller_id: Uuid,
    target_id: Uuid,
    role: GlobalRole,
) -> Result<()> {

    // Ownership is not transferable through this path
    if role == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    // Start transaction
    let mut tx = pool.begin().await?;

    let caller_role = db::global_role(&mut tx, caller_id).await?;

    // Is the caller ranked high enough to change other's roles
    let permissible_roles = [GlobalRole::Owner, GlobalRole::Admin];
    if !permissible_roles.contains(&caller_role) {
        return Err(AppError::Forbidden);
    }

    // Can't change owner's role
    let target_role = db::global_role(&mut tx, target_id).await?;
    if target_role == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    // Admins can't change each other's roles
    if caller_role == GlobalRole::Admin && target_role == GlobalRole::Admin {
        return Err(AppError::Forbidden);
    }

    // No row updated, so the target does not exist
    if !db::set_global_role(&mut tx, target_id, role).await? {
        return Err(AppError::NotFound);
    }

    tx.commit().await?;

    Ok(())
}
