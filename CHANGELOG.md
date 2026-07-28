# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- WebAudio-style runtime with `AudioContext` and `AudioBuffer` support
- `InnerAudioContext` API compatible with mini-game style
- Audio decoders for MP3, OGG, and WAV formats
- Audio streaming and caching pipeline
- Canvas 2D rendering API
- WebGL rendering API
- File I/O operations (sync and async)
- Network fetch API
- Touch input handling
- Android platform support with JNI bindings
- Android demo project ([migo-examples](https://github.com/minigame-labs/migo-examples), `android-java/`)

### Changed
- Renamed project from `minigame_host` to `migo`
- Renamed SO library from `libminigame_host.so` to `libmigo.so`

## [0.1.0] - TBD

Initial public release (planned).

---

## Versioning

This project uses [Semantic Versioning](https://semver.org/):

- **MAJOR**: Incompatible API changes
- **MINOR**: New functionality (backward compatible)
- **PATCH**: Bug fixes (backward compatible)

### Pre-1.0 Policy

While the version is below 1.0.0:
- MINOR version bumps may include breaking changes
- PATCH version bumps are backward compatible

[Unreleased]: https://github.com/minigame-labs/migo/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/minigame-labs/migo/releases/tag/v0.1.0
