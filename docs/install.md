# Installing BrainWashed

BrainWashed is one command, `brainwashed`. Install it with one line:

- **Windows** (PowerShell): `irm https://raw.githubusercontent.com/ahmadalshouly/brainwashed/main/install.ps1 | iex`
- **macOS and Linux**: `curl -fsSL https://raw.githubusercontent.com/ahmadalshouly/brainwashed/main/install.sh | sh`

Builds exist for Windows (x64, and Arm through emulation), macOS (Apple silicon and Intel) and Linux (x64 and Arm). Run the same line again to update.

## First start

The installer starts BrainWashed right away. Later, run `brainwashed` in any terminal. It:

1. Opens the **admin page** in its own browser window. This computer's browser is an admin automatically.
2. Downloads a small model that fits your computer the first time, and the matching llama.cpp runtime. A 2-4 GB model takes a few minutes. The **Models** page shows progress and suggests others.
3. Opens a free, secure tunnel so phones and other computers can reach it **from anywhere**, with no router setup. The terminal and the **Remote access** page show the address.

Then open **Chat**, or press Enter in the terminal for a QR code to scan with your phone.

You need about 8 GB of memory for small models (1-4B parameters), 16 GB for 7-8B models.

## Commands

| Command | What it does |
|---|---|
| `brainwashed` | Start, or open the admin page if it's already running |
| `brainwashed open` | Open the admin page of the running BrainWashed |
| `brainwashed status` | Is it running, which model, which addresses |
| `brainwashed stop` | Stop it |
| `brainwashed service install` | Start it in the background whenever you log in |
| `brainwashed service uninstall` | Stop doing that |
| `brainwashed remote quick \| cloudflare <token> <url> \| url <url> \| relay <url> \| off` | How devices reach it from anywhere ([remote-access.md](remote-access.md)) |
| `brainwashed chat` | Chat in the terminal |
| `brainwashed models`, `pull <model>`, `use <model>` | Manage models from the terminal |
| `brainwashed skills` | List skills and show their folder |

Options: `--port <port>` (default 47860), `--no-browser`, `--local-only` (no tunnel this time), `--data-dir <dir>`.

## Keep it running

`brainwashed service install` registers BrainWashed to start at login and starts it now: a systemd user service on Linux, a launch agent on macOS, a scheduled task on Windows. Open the admin page any time with `brainwashed open`. On Linux, `sudo loginctl enable-linger $USER` keeps it running while you're logged out, which suits a home server.

## Updates

BrainWashed checks GitHub for new releases and the admin page's **Overview** says when one is out. To update, run the install line again. Turn the check off in **Settings**; it sends nothing about you or your computer.

## Windows firewall

The first time BrainWashed serves, Windows may ask whether it may use the network. Allow **Private networks** so phones on your Wi-Fi can connect directly. Remote access through the tunnel works either way.

## Windows Smart App Control

On some Windows 11 PCs, mostly fresh installs, **Smart App Control** is on. It blocks programs that aren't signed and that Microsoft hasn't seen often enough. BrainWashed runs models with [llama.cpp](https://github.com/ggml-org/llama.cpp), whose Windows builds aren't signed and change several times a day, so Smart App Control can stop it. Loading a model then fails with:

```
llama-server exited with exit code: 0xc0e90002 while loading the model
```

To check, open **Windows Security > App & browser control > Smart App Control settings**. If it says **On**, switch it to **Off** and load the model again. To see exactly what was blocked, run this in PowerShell:

```powershell
Get-WinEvent -LogName "Microsoft-Windows-CodeIntegrity/Operational" -MaxEvents 100 |
  Where-Object Id -in 3033,3077 | Select-Object -First 5 TimeCreated, Message | Format-List
```

If you'd rather keep Smart App Control on, you can still chat through a [cloud model](../README.md#cloud-models). Those don't run llama.cpp on your computer.

## Uninstalling

Run `brainwashed service uninstall` if you used it, then delete the binary (`~/.local/bin/brainwashed`, or `%LOCALAPPDATA%\Programs\BrainWashed` on Windows). Models, skills, devices and settings stay in the data folder until you delete it:

- macOS: `~/Library/Application Support/org.brainwashed.host`
- Windows: `%APPDATA%\org.brainwashed.host`
- Linux: `~/.local/share/org.brainwashed.host`
