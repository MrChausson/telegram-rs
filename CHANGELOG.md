# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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