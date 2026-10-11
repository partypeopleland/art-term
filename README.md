# art-term

[繁體中文](README.zh-TW.md)

art-term is a Windows and Android app for developers who want to use Linux shells and coding CLIs from a desktop or phone. Shells and tools run on the remote Linux host; the app manages connections and presents the terminal.

## Features

- Multiple Shell tabs with bounded recovery. After a temporary disconnect, art-term can resume the original session; it does not replay input whose result is uncertain.
- Optional shared desktop preview. The app displays screenshots of the host's shared desktop; it is not an interactive remote desktop.
- A shared Windows and Android connection core, with password-encrypted connection export and import for moving settings between devices.
- Experimental Android voice input. On-device recognition can work offline when the required language data is available. Pauses do not end recording, and finished text is sent to the tab where recording began. Tested only on Pixel 9 so far; English technical terms, commands, and paths can be misrecognized.

## How it works

```text
Windows / Android app
   | HTTPS: Shell management; WSS: terminal and screenshot data
   v
Official openab-pty remote Runtime
   +--> work Shell --> commands / coding CLI
   +--> temporary screenshot Shell --> Host helper --> optional instance-mcp
                                                            |
                                                       shared desktop
```

art-term and openab-pty are independent projects. This app works with the official openab-pty Runtime, which manages remote Shells and carries terminal data. The optional desktop preview also needs a Host helper and instance-mcp. The app does not install these remote services or coding CLIs.

## Getting started

Prepare a Linux host reachable from your Windows PC or Android device over the same private Tailnet, such as Tailscale, and have the official openab-pty Runtime configured. Add its URL and management credential in the app. A coding CLI is optional and must already be installed on the host. For desktop previews, also prepare a shared desktop, the Host helper, and instance-mcp.

## Documentation and builds

The detailed documentation is currently written in Traditional Chinese. Start with the [quick start](docs/openab-host-setup.md) or [documentation index](docs/INDEX.md). See also [architecture](docs/refarch/architecture.md), [Shell sessions and recovery](docs/refarch/sessions.md), [desktop preview](docs/refarch/desktop.md), [connection transfer](docs/refarch/connection-transfer.md), the [Windows build guide](docs/platforms/windows.md), the [Android build guide](docs/platforms/android.md), and the [Android voice-input decision](docs/bdr/BDR-001-android-voice-input.md).

## Issues and license

Issues are open, and bug reports are welcome. Do not post credentials, host addresses, or other private information. This project is MIT licensed; third-party components retain their own license terms. See [LICENSE](LICENSE) and [third-party notices](docs/third-party-notices.md).