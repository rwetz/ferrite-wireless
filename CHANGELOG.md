# Changelog

All notable changes to Wireless are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.3.1] - 2026-10-09

### Changed

- Add a live output oscilloscope and restore saved appearance at launch.
- Include current Ferrite layout, window memory, and overlay fixes.

## [0.2.0] - 2026-10-07

### Added

- Every year-round SomaFM channel on the dial: 37 stations, 0.5 MHz apart
  from 88.1, grouped ambient → electronica → lounge and soul → rock and
  pop. Reception is tighter to match, so there is still static between
  them. The holiday channels and SomaFM Live/Specials are left off: they
  are silent most of the year. Fluid moved to 93.1 (the new default).
- The dial, meters and ticker are as wide as the window; the frequency
  goes up a size on a big one. The set sits centered in spare height.
- The window reopens at the size, place and state it was closed in.

### Fixed

- The scheme picker in Settings opens (its menu was drawn under the
  drawer; fixed in ferrite-design).
- The first window is wide enough for the dial at 150% scaling, and fits
  the screen.

## [0.1.1] - 2026-10-07

### Fixed

- Windows: the Settings button in the title bar does something when clicked. The
  whole title bar was a window-drag area, so Windows took the click as the
  start of a drag (ferrite-design's `title_bar`).
- The date rolls over at local midnight, not UTC midnight.

## [0.1.0] - 2026-10-07

The first release.

### Added

- An FM-style dial with stations on it, a text scale and needle, seek that
  sweeps through the static between stations, and a settle-then-lock tuner.
- Generated radio static that the station fades up through as the signal
  strengthens, and an RF gauge.
- A live two-channel VU meter.
- The station, genre and current track (ICY metadata) on a marquee.
- MP3, AAC and Ogg Vorbis streams over HTTP(S); eight SomaFM presets, or
  your own `station` lines.
- The sound cuts as the window closes with the CRT switch-off.
- The shared look from Lodestone (`FERRITE_*`) when launched from it.
- The pixel-art logo as the Windows executable, window and taskbar icon.

[0.1.1]: https://github.com/rwetz/ferrite-wireless/releases/tag/v0.1.1
[0.1.0]: https://github.com/rwetz/ferrite-wireless/releases/tag/v0.1.0
