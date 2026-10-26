# Migrate from 1.x or Preview to 2.0

NyaTerm 2.0 uses a native GPUI desktop interface. Choose a package matching your operating system and CPU architecture; these instructions do not promise an in-place architecture change. Windows 10 is the minimum requirement; legacy Windows 7 installations are not supported.

## Back up before migrating

Quit NyaTerm completely, then copy the entire data directory and related key files to a separate location. Stable installed builds normally use `.nyaterm/` in your user directory; portable builds use `data/` beside the program. Backing up the entire portable directory also preserves programs, markers, and key files. Keep your original installer.

If you have a master password, also export an encrypted `.nya` backup. An exported backup does not replace a complete directory copy made while the application is closed. Do not attach keys, credentials, or real user data to issue reports.

## Windows portable builds

1. Quit NyaTerm and back up the entire old portable directory.
2. Download the complete 2.0 portable ZIP for the same architecture and extract it to a temporary directory.
3. Copy all extracted program files into the old directory, replacing matching files. Preserve the existing `data/` directory and key files; do not overwrite them with package contents.
4. Include `nyaterm-rdp-helper.exe`, `nyaterm-vnc-helper.exe`, the MCP program, and the complete ConPTY directory. Replacing only `NyaTerm.exe` is insufficient.
5. Start the application, verify its version, connections, credentials, and master password, then test RDP/VNC.

**The 1.x portable updater cannot perform this migration.** It uses a file allowlist and does not deploy the new helpers. Use the manual procedure above if the updater fails. Subsequent 2.0 portable updates support replacing the full program payload while preserving data.

## Windows MSI and NSIS installations

For an old MSI installation, back up first, uninstall the old application through Windows Installed Apps, then install the same-architecture 2.0 `-setup.exe`. Do not select any option that removes user data. Verify settings, shortcuts, the installation directory, and uninstall entries so that two confusing installation entries do not remain.

Automatic MSI-to-NSIS migration is not promised, and an EXE must not be treated as an MSI update payload. In-place upgrades from old NSIS releases require validation for that version. Until support is confirmed, use the same backup, uninstall, and install procedure.

## macOS and Linux

For a manually installed macOS application, quit and replace `/Applications/NyaTerm.app`, retaining the user data directory. Homebrew users should run `brew upgrade --cask nyaterm`.

On Linux, quit before replacing an AppImage with one for the same architecture and preserve its executable permission. For deb/rpm installations, update the complete package using the appropriate package manager rather than replacing individual managed files.

GPUI checks signed update manifests and offers download and installation actions for supported Windows NSIS/portable installations, manually installed macOS bundles, and Linux AppImages. Homebrew, deb/rpm, and unrecognized installations use their package manager or manual updates.

## Move from Preview to Stable

Preview and Stable use separate application identities and data directories (`.nyaterm-preview/` and `.nyaterm/`). Set a master password in Preview and export a `.nya` backup. Install Stable, restore that backup, restart, and verify it. Do not merge the two data directories or assume Preview will switch to the stable channel automatically.

## Verify and recover

Check connections, password and private-key credentials, the master password, known hosts, OTP, backup restoration, and cloud sync. Confirm local data before resuming automatic sync; never let a test instance write to your production sync copy.

If migration fails, quit completely and restore both programs and the complete data copy from before migration. Do not simply replace the executable with 1.x and let it open data already written by 2.0. Retain backups until recovery and ordinary use have been verified.
