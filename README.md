# osdhub-manager

Manages the games of an [OSDHub](https://github.com/higorhgon/osdmenu) device from a computer (the SD card of an MMCE
device, a USB drive...), so OPL Manager (Windows only) isn't needed, from a terminal interface or with commands:

- **list** — the PS2 and PS1 games on the device, with the title IDs read from their discs
- **covers** — downloads the case covers (`COV`) and discs (`ICO`) of the games into `ART/`, named like OPL does
  (`ART/SLUS_202.12_COV.jpg`), for OSDHub's game covers (`games_covers = 1`) and OPL
- **rename** — renames the PS2 ISOs to OPL's `<title ID>.<name>.iso` form (`SLUS_202.12.BLOODY ROAR 3.iso`),
  which OPL needs to find the game's art, configuration and cheats

It's a single executable without dependencies: download it from the
[releases](https://github.com/higorhgon/osdhub-manager/releases) (Linux, Windows and macOS) and run it from a terminal.

## Terminal interface

```sh
osdhub-manager /run/media/$USER/MMCE
```

The device root is the folder where the device is mounted, which holds the `CD`, `DVD`, `EMBER` and `ART` folders.
Without it (`osdhub-manager`, or opening the executable from a file manager), a folder browser opens first. It starts in
the folder where the desktop mounts the removable drives (`/run/media/$USER`, `/media/$USER` or `/Volumes`) and marks
the folders that look like an OSDHub device (`OSDHub: CD DVD ART`):

| Key | |
| --- | --- |
| `↑` `↓` / `Enter` / `←` | Move / open the folder / go back |
| `s` | Uses the selected folder as the device root (or `✓ Use this folder` for the folder being browsed) |
| `~` | Home folder |

When the folder picked doesn't have the folders OSDHub uses (`ART`, `CD`, `DVD` and `EMBER/games`, or the ones given with
`--cd-folder`/`--dvd-folder`), it offers to create them, which prepares a new SD card or USB drive for OSDHub.
The games are shown in a table with their title ID, whether their case cover (`COV`) and disc (`ICO`) are in `ART/`,
and whether the game should be renamed: the PS2 ISOs without OPL's name show `rename` (`folder` for ISOs in their own
subfolder, which OPL doesn't list), and the names that don't fit on OSDHub's menu show `⚠`, with the part OSDHub cuts
in yellow:

```
┌ Games (4) ───────────────────────────────────────────────────────────┐
│      Title ID    COV ICO Rename   Name                               │
│● PS2 SLUS_202.12 ✓   ✗   rename   Bloody Roar 3                      │
│  PS2 SLUS_206.80 ✓   ✓   ⚠        HARVEST MOON - SAVE THE HOMELAND   │
│  PS2 SCUS_973.28 ✓   ✓   ✓        Gran Turismo 4                     │
│  PS1 SCUS_949.00 ✗   ✗   ✓        Crash Bandicoot (USA)              │
└──────────────────────────────────────────────────────────────────────┘
```

When the terminal is at least 100 columns wide, the case cover and the disc of the selected game are shown on the right.
Terminals that show images (kitty, Ghostty, WezTerm, foot, Konsole, iTerm2...) draw them as images, through kitty's
protocol, Sixel or iTerm2's; the others (like Alacritty) draw them with colored half blocks. The terminal is asked which
one it supports when the interface opens; `--no-images` skips that and uses half blocks.

| Key | |
| --- | --- |
| `↑` `↓` `PgUp` `PgDn` | Move |
| `Space` / `a` | Mark the game / mark all (the actions apply to the marked games, or to all the games shown) |
| `Tab` | All games, PS2 only or PS1 only |
| `c` | Downloads the art of the games, in the background, with the progress in the log |
| `t` / `f` | Art types to download (COV+ICO, COV, ICO) / downloads them again even when they're in `ART/` |
| `r` | Renames the selected game, or the marked ones one after the other, in a name editor (below) |
| `s` | Reads the games again |
| `q` | Quits |

### Renaming games

`r` opens an editor for the name of the game. `Enter` renames it, `Tab` skips it and `Esc` stops renaming.

- **PS2**: only the name of the ISO is edited: the title ID before it and the extension after it stay
  (`SLUS_202.12.` `Bloody Roar 3` `.iso`), so the file always gets OPL's form.
- **PS1**: the name of the game folder in `EMBER/games/` is edited. It's optional: Ember runs the games with any folder
  name, so the editor only says whether the current name fits on OSDHub's menu. When the title ID was taken from the
  folder name (the disc has no `SYSTEM.CNF`), the editor warns if the new name drops it, since the art is found by it.

While the name is edited, the editor shows whether it fits on OSDHub's menu. OSDHub shortens the names that don't
fit between the cover panel and the right edge of the screen with `...`, so the editor shows the name as OSDHub would
(`⚠ OSDHub shows "HARVEST MOON - SAVE THE HOME..."`) and the characters it cuts in yellow, like the game table does.
The width of a name is estimated from the widths of OSDSYS's font, for the menu position given with `--menu-x`
(OSDHub's `OSDSYS_menu_x`, 400 by default) and with the cover panel (`--no-covers` when `games_covers = 0`, which
leaves the whole width of the screen for the names). OSDHub also keeps only the first 79 characters of a name.

The editor doesn't accept names that OPL wouldn't list (ISOs over 160 characters) or OSDHub (PS1 folders over 127
bytes), that FAT and exFAT don't allow (`/ \ : * ? " < > |`), nor the name of another game. OSDHub keeps the
favorites and play counts of the games by their paths, so those of a renamed game start over; refresh the game lists
in OSDHub after renaming.

## Commands

```
osdhub-manager <COMMAND> <DEVICE ROOT> [OPTIONS]
```

For scripts, the same actions are available as commands:

```sh
# The games and their title IDs
osdhub-manager list /run/media/$USER/MMCE

# Covers and discs of every game
osdhub-manager covers /run/media/$USER/MMCE

# Only the PS1 case covers, showing what would be downloaded first
osdhub-manager covers /run/media/$USER/MMCE --ps1 --types cov --dry-run

# Shows how the PS2 ISOs would be renamed (and the names that don't fit on OSDHub), then renames them
osdhub-manager rename /run/media/$USER/MMCE
osdhub-manager rename /run/media/$USER/MMCE --apply
```

After downloading covers or renaming ISOs, refresh the game lists in OSDHub ("Refresh list"), which converts the new
covers and picks up the new ISO paths.

| Option | |
| --- | --- |
| `--ps1` / `--ps2` | Only PS1 or PS2 games (both by default) |
| `--types cov,ico` | Art types to download (both by default) |
| `--force` | Downloads the images again even when they're already in `ART/` |
| `--dry-run` | Shows what would be downloaded without downloading |
| `--apply` | Renames the ISOs (`rename` only shows the changes without it) |
| `--cd-folder DIR` / `--dvd-folder DIR` | PS2 folders (`CD` and `DVD` by default), like OSDHub's `games_cd_folder`/`games_dvd_folder` |
| `--oplm-url URL` | OPL Manager's art database (`none` to skip it) |
| `--no-xlenore` | Doesn't use xlenore's cover collections |
| `--no-images` | Draws the art previews with colored half blocks instead of asking the terminal for images |
| `--menu-x N` | Center of OSDHub's menu (`OSDSYS_menu_x`, 400 by default), to find the names that don't fit |
| `--no-covers` | OSDHub doesn't show covers (`games_covers = 0`), which leaves more room for the names |

## Games

The games are found the same way OSDHub's launcher finds them:

- **PS2**: `*.iso` files in the `CD` and `DVD` folders, or subfolders of them holding exactly one `*.iso`
  (OSDHub lists those, but OPL doesn't, so `rename` leaves them as they are)
- **PS1**: `EMBER/games/<game>/` folders holding a `*.cue` file, the games OSDHub runs with Ember

The title ID comes from the `SYSTEM.CNF` file on the disc (`BOOT2 = cdrom0:\SLUS_202.12;1` on PS2,
`BOOT = cdrom:\SCUS_949.00;1` on PS1), read from the ISO or from the first track of the CUE/BIN, as the console does.
Early PS1 discs without `SYSTEM.CNF` are named after their executable (`SLPS_000.01`). When the disc can't be read,
an ID in the file or folder name is used.

## Art sources

Tried in order, for each game and art type:

1. **OPL Manager's art database**, which OPL Manager downloaded from its own server until it was shut down, from
   [its dump on GitHub](https://github.com/higorhgon/psx-ps2-opl-art-database) (a fork of
   [Luden02's](https://github.com/Luden02/psx-ps2-opl-art-database)) (`PS1/<ID>/<ID>_COV.png`, `PS2/<ID>/...`),
   with every OPL art type, including the discs (`ICO`). Its backup on archive.org has the same layout, and can be used
   instead with `--oplm-url https://archive.org/download/OPLM_ART_2024_09/OPLM_ART_2024_09.zip`
2. **xlenore's [PS1](https://github.com/xlenore/psx-covers) and [PS2](https://github.com/xlenore/ps2-covers) cover
   collections** (the ones DuckStation and PCSX2 use), for the case covers

Images already in `ART/` (as `.jpg` or `.png`) aren't downloaded again unless `--force` is used.

## Building

```sh
cargo build --release
```

The dependencies are [ratatui](https://ratatui.rs), crossterm and [ratatui-image](https://github.com/ratatui/ratatui-image)
with [image](https://github.com/image-rs/image) (JPEG and PNG only) for the terminal interface, and
[ureq](https://github.com/algesten/ureq) (HTTPS with rustls), pinned by `Cargo.lock`. The release
binaries are built by GitHub Actions when a `v*` tag is pushed: a static Linux binary (musl), Windows and macOS.
