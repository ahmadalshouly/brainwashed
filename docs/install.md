# Installing BrainWashed

Download the installer for your computer from the [latest release](https://github.com/ahmadalshouly/brainwashed/releases).

| Your computer | File to download |
|---|---|
| Mac with Apple silicon (M1 or later) | `BrainWashed_<version>_aarch64.dmg` |
| Mac with an Intel processor | `BrainWashed_<version>_x64.dmg` |
| Windows 10 or 11 | `BrainWashed_<version>_x64-setup.exe` (or the `.msi`) |
| Ubuntu, Debian, Mint | `brainwashed_<version>_amd64.deb` |
| Fedora, openSUSE | `BrainWashed-<version>-1.x86_64.rpm` |
| Any other Linux | `BrainWashed_<version>_amd64.AppImage` |

Not sure which Mac you have? Apple menu > About This Mac: "Chip: Apple M…" means Apple silicon.

## First start

1. Open BrainWashed. The **Models** tab suggests models that fit your computer's memory.
2. Pick one and click **Download**. A 2-4 GB model takes a few minutes on a typical connection. BrainWashed also downloads the matching llama.cpp runtime for your hardware the first time.
3. When the model is loaded, open **Chat**.

You need about 8 GB of memory for small models (1-4B parameters), 16 GB for 7-8B models.

## Until the installers are signed

Early releases may not be signed with an Apple or Microsoft certificate yet, so your computer warns you the first time:

- **macOS:** "BrainWashed can't be opened because Apple cannot check it for malicious software." Open System Settings > Privacy & Security, scroll down and click **Open Anyway** next to BrainWashed. You only need to do this once.
- **Windows:** SmartScreen says "Windows protected your PC". Click **More info**, then **Run anyway**.
- **Linux:** make the AppImage executable (`chmod +x BrainWashed_*.AppImage`) before running it.

## Updates

BrainWashed checks GitHub for new releases when it starts and shows **Version X is available** at the bottom of the sidebar. Click it to open the download page. It sends nothing about you or your computer; you can turn the check off with the **Check for updates** box.

## Using it from other devices

- At home: open **Devices** and pair a phone or another computer with the QR code. See the [README](../README.md#using-it-from-your-phone-or-another-computer).
- On Windows, the first time you turn on **Allow phones and browsers on this network**, Windows Firewall asks whether BrainWashed may use the network. Tick **Private networks** and click **Allow access**. Your Wi-Fi must also be set to a private network (Settings > Network & internet > Wi-Fi > your network > **Private network**), or phones can't connect.
- Away from home: set up a relay. See [relay.md](relay.md).

## Uninstalling

Remove the app as usual for your system. Your models, skills and settings stay in the app data folder until you delete it:

- macOS: `~/Library/Application Support/org.brainwashed.host`
- Windows: `%APPDATA%\org.brainwashed.host`
- Linux: `~/.local/share/org.brainwashed.host`
