![project logo](./logo.png)

## Description
A simple chat application for use in small to medium sized groups. It is a single
server instance with many room types (public/private/hidden). Unlike discord clones,
this is not an ecosystem server but a single "guild" instance with rooms acting
as separators for groups.

Public rooms are meant to be open to anyone on the server to join and leave as they please.
Locked rooms are meant to be visible for search but require a member of the room to enter.
Hidden rooms are as the name implies, hidden from discoverability and requires invite to enter, these can function as a DM (ie. one on one hidden room between to users).

Using SQLite for making database mangement simple and not requiring an active service to run (ie. Postgres).

## Basic Security
|Category| Method | Description | 
|-|-|-|
| Invite Codes (12 Characters by Default) | OsRng | Invite is required by an Admin to enter server | 
| Password Hashing| Argon2id | Prevents password snooping (no plain-text) |
| Session Tokens | OsRng + SHA256 | Randomly generated token per login session | 

## Configuration
A `config.toml` is written alongside server on first run with defaults.
It covers:
- Bind address
- Database (.db) location
- Uploaded "files" location
- Length of session lifetime
- Limits (such as on usernames, display name, message, etc)

By default the `config.toml` file is expected to be at the same directory as the server.
To use a different directory on boot, set an environment variable `XENON_CONFIG` with the desired location.

## Building
Requires Rust 1.85 or newer (this crate uses edition 2024). Distribution packages
on other OS's (such as Ubuntu Server) are using older packages than that, so install via: [rustup](https://rustup.rs) rather than `apt install cargo`.

SQLite is compiled from source, so a C toolchain is also needed:

    sudo apt install build-essential
    cargo build --release

## Plan
- [x] Create Database model/schema (first pass)
- [x] Get REST API functional
    - [x] Registration, login, registration codes
    - [x] Rooms: create, join, leave, list, discovery
    - [x] Messages: post, paginated fetch, edit, delete, spoilers
- [x] Get sockets functional
    - [x] Live broadcast of messages, edits, deletes, invites, bans
- [x] File uploads and attachments
- [x] Room membership
    - [x] Invites for locked/hidden rooms
    - [x] Bans
    - [x] Per-room permissions and global roles
- [ ] Room settings (rename, delete)
- [ ] Profile and account (edit profile, change password, delete account, transfer ownership)
- [ ] Messages
    - [ ] Search
    - [ ] Read state
- [ ] Online presence and typing indicators
- [ ] Game presence (Xbox/Steam)
- [ ] Rate limiting
- [ ] Create rudimentary GUI Application (Slint)
    - [ ] Linux/Windows Build
    - [ ] Android Build
- [ ] Create PWA Web App (Typescript) for iOS

## Developer Comment(s):
I am not a database, server, or security developer; so have no expectation of a professional grade chat
application. Although I work in software development I primary work on the GUI side (Qt) and video game space.

I wanted a project to learn Rust on and have a useful tool my friends can use. We were using
[Spacebar](https://github.com/spacebarchat) + [Fermo](https://git.sovrahi.com/oh64/Fermo) previously, so if you are looking
to move away from Discord like we did I'd suggest looking at those projects. This project is meant to be similar
in concept to those (self-hostable private chat app), but less of a direct discord clone.

My plan for this project is something very easy to setup, deploy, and maintain.
Currently the only thing required to get it running is compiling the program and running the executable.
The config files, file upload path, database file, and defaults are all setup on first run.

_Set expectations accordingly_

## License:
TBD