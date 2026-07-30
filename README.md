# Xenon

## Description
A simple chat application for use in small to medium sized groups. It is a single
server instance with many rooms (public/private) and DM support. Unlike discord clones,
this is not an ecosystem server but a single "guild" instance with rooms acting
as separators for groups.

Using SQLite for making database mangement simple and not requiring an active service to run (ie. Postgres).

Basic Security for:
|Category| Method |
|-|-|
| Invites Codes (8 Characters by Default) | OsRng | 
| Password Hashing| Argon2id |
| Session Tokens | OsRng + SHA256 |


## Plan
- [x] Create Database model/schema (first pass)
    - [x] Owner user created at first launch
    - [x] Create `chat.db` at first launch
- [] Get REST API functional
- [] Get sockets functional
- [] Create rudimentary GUI Application (Slint)
    - [] Linux/Windows Build
    - [] Android Build
- [] TBD

## License:
TBD