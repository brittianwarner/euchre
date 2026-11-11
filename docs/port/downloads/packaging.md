# Desktop packaging

`scripts/release-desktop.sh` builds all three downloads on an Apple-silicon Mac and writes `downloads/` plus `manifest.json`.

- Mac: universal (arm64 + x86_64) `Euchre-<v>-mac.dmg`, built natively, ad-hoc signed, macOS 13 or newer.
- Windows: `Euchre-<v>-windows.zip` holding `Euchre.exe` (x86_64, GUI subsystem, icon and version resource), cross-built with mingw-w64 in an arm64 Debian 11 container.
- Linux: `Euchre-<v>-linux.AppImage` (x86_64, glibc 2.31 or newer), built in the same container.

The app opens `EUCHRE_DEFAULT_SERVER_URL` (default https://euchre.sh) unless `EUCHRE_SERVER_URL` or `EUCHRE_DESKTOP_BACKEND` overrides it.

`scripts/release-desktop.sh --upload` then uploads `files/<file>` (immutable) and `manifest.json` (last, no-cache) to the Railway bucket `euchre-downloads`.

## Unsigned: one-time open step per OS

- macOS: ad-hoc signed only. Open the dmg, drag Euchre to Applications, then right-click Euchre and choose Open, then Open again.
- Windows: not signed. SmartScreen shows "Windows protected your PC": choose More info, then Run anyway.
- Linux: `chmod +x Euchre-*.AppImage`, then run it.
