use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::UserSummary;

/// Looks up a user's public profile by id.
///
/// Not filtered by deleted_at: a soft-deleted user's past messages still need
/// a name to display, so their profile stays resolvable after they're gone.
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
