use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub global_role: i64,
    pub created_at: i64,
}

/// !!! Permanent !!!
/// Never reuse or renumber a retired variant's number once rows exist
/// Encodes as integer in DB
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[repr(i16)]
pub enum GlobalRole {
    Owner = 0,
    Admin = 1,
    Member = 2
}