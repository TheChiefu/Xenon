use uuid::Uuid;

use crate::error::Result;

/// Whether the user has a ban on the room that has not expired
/// - conn: Connection to SQL DB
/// - room_id: Room the ban applies to
/// - user_id: User the ban is against
/// - now: Time the expiry is measured against
pub async fn is_banned(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    user_id: Uuid,
    now: i64,
) -> Result<bool> {

    let exists: bool = sqlx::query_scalar(
        "
        SELECT EXISTS(
            SELECT 1
            FROM room_bans
            WHERE room_id = ?1 AND user_id = ?2 AND (expires_at IS NULL OR expires_at > ?3)
        )
        "
    )
    .bind(room_id)
    .bind(user_id)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;

    Ok(exists)
}
