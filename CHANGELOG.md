# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [v0.2.0] - 2026-08-16

### Added
- In-app sign-in: the released binary can log in by itself (phone → code →
  two-step verification), no console helper or Rust toolchain required.
- Per-user data directory (`.env`, session and cache) so the installed binary
  works from anywhere; `TG_DATA_DIR` overrides it.
- API credentials are embedded at build time from the `TG_API_ID` /
  `TG_API_HASH` Action secrets, overridable via `.env` — the first launch
  shows a branded login screen instead of a dead end.

## [v0.1.0] - 2026-08-16

First public release of **tg**, a minimal Telegram client written in Rust,
rendered natively with tiny-skia and winit (no GPU or browser engine).

### Added
- Chat list showing real profile photos for users, groups, and channels,
  downloaded and cropped into round avatars.
- Text and photo message bubbles; click a photo to view it full screen.
- Local session storage via the telegram-rs low-level API (`.tg.session`).
- Release automation: Linux build, test suite, tarball and AppImage, tagged
  GitHub releases with assets (`v0.1.0`, 2026-08-16).