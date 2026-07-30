![project logo](./logo.png)

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

## Developer Comment(s):
I am not a database, server, or security developer; so have no expectation of a professional grade chat
application. Although I work in software development I primary work on the GUI side (Qt) and video game space.

I wanted a project to learn Rust on an have a useful tool my friends and I can use. We are primarily using
[Spacebar](https://github.com/spacebarchat) + [Fermo](https://git.sovrahi.com/oh64/Fermo), so if you are looking
to move away from Discord like we did I'd suggest looking at those projects. This project is meant to be similar
in concept to those (self-hostable private chat app), but less of a direct discord clone. 

_Set expectations accordingly_

## License:
TBD