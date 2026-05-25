# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Driver
- **Soft-off mode** — press `Shift + Maschine` to blank the lights and screen and silence all output without quitting the driver; press again to restore the previous state. Useful for stepping away from the controller without unplugging it.
- **Per-control configuration** — every pad, button, encoder, and slider action can now be customised independently in `config.toml` (note number, MIDI channel, CC number, behaviour). Pads are keyed by physical pad number (1–16); buttons by their printed names.
- **Pad velocity curves** — shape how hard hits translate to MIDI velocity (linear, soft, hard).
- **Pad aftertouch** — pad pressure is now sent as polyphonic aftertouch when configured.
- **Slider touch events** — the touch strip's touch-on / touch-off can be mapped to a note or CC, independent of the position value.
- **Hardware preferences** — device-side settings (pad sensitivity, screen brightness) can be configured.
- **Generated reference config** — a `default_config.toml` containing every available setting is checked in and used by docs and the `-c` flag.

#### Slider LED Rendering
- New `[slider.led]` settings: choose `bar`, `pan`, or `dot` rendering, with an optional stylized trail (leading led brighter than the rest).
- Slider LEDs auto-blank after a configurable idle delay (default is 5s).
- Rainbow slider sweep added to the self-test.

### Changed
- **Configuration file format is new** — flat keys have been replaced by a nested per-control schema. Existing `config.toml` files from 0.4.0 will not load as-is; see `default_config.toml` for the new layout.
- Internal restructure of the driver into smaller modules and a shared library crate; no user-visible behaviour change beyond what's listed above.
- Driver now blanks the lights and screen when it exits (e.g. on `Ctrl+C`) instead of leaving the controller frozen in its last state.

### Documentation
- README rewritten around the userspace HID driver architecture and updated for the new per-control configuration schema.
- `example_config.toml` replaced by the generated `default_config.toml`.

### Infrastructure
- CI now builds with Node.js 24 to match `bitwig/.nvmrc` and the `engines.node` constraint (`>=24 <25`); previously `npm ci` failed under `engine-strict=true`.

## [0.4.0] - 2026-01-20

### Added

#### Driver
- **SysEx screen control protocol** - DAW can now send text to the OLED screen
  - Command 0x01: Display text (centered, auto-truncated)
  - Command 0x02: Clear screen
  - Manufacturer ID: `F0 00 21 09 <cmd> <data...> F7`
- Screen state management with dirty flags for efficient updates
- Support for MIDI input to control screen via SysEx messages

#### Bitwig Controller Script
- **Complete modular rewrite** with modern ES6 module architecture
  - Separated concerns into features, handlers, modes, and utilities
  - Built with Rollup for optimized single-file output
  
- **Mode System** - Four operational modes with visual indicators
  - **Play Mode**: Standard MIDI note performance
  - **Step Mode**: 16-step sequencer with drum pad name display
  - **Clip Mode**: 4x4 clip launcher for triggering clips and scenes
  - **Mixer Mode**: Track select, mute, solo, and arm controls
  
- **Note Repeat** - Auto-retriggering of held pads
  - Toggle on/off
  - Adjustable rate (1/16, 1/8, 1/4 notes)
  - Visual feedback on button LED
  
- **Fixed Velocity** - Force consistent velocity for all pad hits
  - Toggle on/off  
  - Configurable fixed velocity value (default: 100)
  
- **Step Sequencer** features
  - Edit any MIDI note (0-127) with visual feedback
  - Display drum pad names from Bitwig (e.g., "Kick", "Snare")
  - Visual playhead with white LED
  - Yellow LEDs for active steps
  - Encoder controls note selection
  - Clear all steps function
  
- **Screen Integration**
  - Real-time display of current mode
  - Track name in Play and Mixer modes
  - Note/drum pad name in Step mode
  - Feature status notifications
  
- **Enhanced Playback Feedback**
  - Configurable colors for manual hits and clip playback
  - Mode-aware LED updates (only active in Play mode)
  - Separate preferences for manual and playback feedback
  
- **User Preferences**
  - Enable/disable playback and manual hit feedback
  - Choose between track color or fixed color for playback
  - Customize colors for manual hits and playback

### Changed

#### Driver
- **Removed automatic blue LED feedback on pad touch**
  - Eliminates LED conflicts with DAW-controlled states
  - Pads now controlled exclusively via MIDI from controller scripts
  - Fixes flickering and state synchronization issues
- Refactored main loop to share screen state between MIDI callback and main thread
- Improved screen rendering with centered text alignment

#### Bitwig Controller Script
- Migrated from single monolithic file to modular architecture
- Improved LED update efficiency with debouncing and state tracking
- Better separation of mode-specific logic
- Optimized step sequencer with height=1 clip window for reliability

### Fixed

#### Driver
- Screen updates from MIDI input now properly synchronized
- Race condition between LED control sources eliminated

#### Bitwig Controller Script  
- Step sequencer pads now accurately reflect Bitwig's clip state
- No more blue/yellow LED flickering when pressing pads
- Step data correctly loads when entering step mode or changing notes
- Playback feedback no longer interferes with other modes
- Encoder touch suppression prevents spurious deltas

### Infrastructure
- Updated GitHub Actions workflow to build both driver and Bitwig script
- Added Node.js setup to CI pipeline
- Automated artifact uploads for both driver binary and controller script
- Enhanced release notes with installation instructions for both components

## [0.3.0] - Previous Release

Initial public release with basic MIDI driver functionality + bitwig script - after forking.

[0.4.0]: https://github.com/askz/maschine-mikro-mk3-driver/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/askz/maschine-mikro-mk3-driver/releases/tag/v0.3.0
