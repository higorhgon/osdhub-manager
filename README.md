# osdhub-manager

Manages the games of an [OSDHub](https://github.com/higorhgon/osdmenu) device from a computer (the SD card of an MMCE
device, a USB drive...), so OPL Manager (Windows only) isn't needed, from a terminal interface or with commands:

- **list** — the PS2 and PS1 games on the device, with the title IDs read from their discs
- **covers** — downloads the case covers (`COV`) and discs (`ICO`) of the games into `ART/`, named like OPL does
  (`ART/SLUS_202.12_COV.jpg`), for OSDHub's game covers (`games_covers = 1`) and OPL
- **rename** — renames the PS2 ISOs to OPL's `<title ID>.<name>.iso` form (`SLUS_202.12.BLOODY ROAR 3.iso`),
  which OPL needs to find the game's art, configuration and cheats
- **install** — installs OSDHub in the memory card image an MMCE device boots from (or in a folder to copy to a
  memory card), and RiptOPL, Neutrino and Ember on the device, from their latest releases
- **config** — edits OSDMenu's configuration, `SYS-CONF/OSDMENU.CNF`, right inside the memory card image an MMCE
  device boots from, or in a `.cnf` file

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
The games are shown in a table with their title ID, region, whether their case cover (`COV`) and disc (`ICO`) are in `ART/`,
and whether the game should be renamed: the PS2 ISOs without OPL's name show `rename` (`folder` for ISOs in their own
subfolder, which OPL doesn't list), and the names that don't fit on OSDHub's menu show `⚠`, with the part OSDHub cuts
in yellow:

```
┌ Games (4) ──────────────────────────────────────────────────────────────────┐
│      Title ID    Region COV ICO Rename   Name                               │
│● PS2 SLUS_202.12 USA    ✓   ✗   rename   Bloody Roar 3                      │
│  PS2 SLUS_206.80 USA    ✓   ✓   ⚠        HARVEST MOON - SAVE THE HOMELAND   │
│  PS2 SCUS_973.28 USA    ✓   ✓   ✓        Gran Turismo 4                     │
│  PS1 SCUS_949.00 USA    ✗   ✗   ✓        Crash Bandicoot (USA)              │
└─────────────────────────────────────────────────────────────────────────────┘
```

When the terminal is at least 100 columns wide, the case cover and the disc of the game under the cursor are shown on
the right.
Terminals that show images (kitty, Ghostty, WezTerm, foot, Konsole, iTerm2...) draw them as images, through kitty's
protocol, Sixel or iTerm2's; the others (like Alacritty) draw them with colored half blocks. The terminal is asked which
one it supports when the interface opens; `--no-images` skips that and uses half blocks.

| Key | |
| --- | --- |
| `↑` `↓` `PgUp` `PgDn` | Move |
| `/` | Searches the games (below); `Enter` keeps the search and `Esc` clears it |
| `Space` / `a` | Selects the game / all the games shown (downloads are for the selected games, or for all the games shown when none is selected) |
| `Tab` | All games, PS2 only or PS1 only |
| `c` | Downloads the covers and discs of the games into `ART/`, in the background, with the progress in the log |
| `t` | What to download: covers and discs, covers only or discs only (`Download` in the header) |
| `f` | Whether the images already in `ART/` are kept or downloaded again and replaced (`Images already in ART`) |
| `r` | Renames the game under the cursor, or the selected ones one after the other, in a name editor (below) |
| `s` | Reads the games again |
| `1` / `2` | The Games tab / the Config tab (below) |
| `q` | Quits |

### Searching

`/` searches the games by their system, title ID, region and name, with the words in any order and without case:
`crash ps2` and `PS2 Crash` find the PS2 Crash games, and `crash` the Crash games of both systems. The title IDs are
found in any form (`SLUS_202.12`, `SLUS-20212`, `slus20212` or a part of them), the systems also as `psx`, and the
regions also as `ntsc-u`, `pal`, `europe`, `ntsc-j` or `japan`. A word that isn't there can still match a name with its
letters in order (`crsh` finds Crash), and those games are listed after the others.

The region comes from the title ID: its prefix tells the region of the disc (`SLUS` and `SCUS` are USA, `SLES` and
`SCES` Europe, `SLPM`, `SLPS` and `SCPS` Japan, `SCKA` Korea...). For the games without a title ID, a region in the name
is used, as in Redump's names (`Crash Bandicoot (USA)`).

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

## Configuration

The Config tab (`2`) edits OSDMenu's settings in `OSDMENU.CNF`. On an MMCE device, OSDMenu reads it from
`mc0:/SYS-CONF/OSDMENU.CNF`, a file inside the memory card image the device boots from, so the tab looks for the
memory card images in the `BOOT` folders under `MemoryCards` (`MemoryCards/BOOT/*.mcd`, `MemoryCards/PS2/BOOT/*.mcd`...)
and opens the one with `OSDMENU.CNF`. When there's none or more than one, it lists them, with an option to open any
memory card image (`.mcd`, `.ps2`, `.bin`) or `.cnf` file instead. A memory card without `OSDMENU.CNF` gets one
(and its `SYS-CONF` folder) when saving.

Every setting OSDMenu knows is listed by section, with what it does and its default (in parentheses when it isn't set),
followed by the menu entries and the other settings in the file. Values OSDMenu wouldn't take are marked with `✗`,
and a line that would make OSDMenu stop reading the rest of the file (a line that isn't `name = value` nor a comment)
is reported.

| Key | |
| --- | --- |
| `Enter` / `←` `→` | Toggles a 0/1 setting, goes through the values of a setting with a few (`games_cover_type`: `cov`, `ico`), or types the others |
| `e` | Types the value |
| `d` | Back to the default: the setting's line is commented out |
| `s` | Shows the changes (`-` old line, `+` new line) and saves them after asking |
| `u` | Undoes the changes not saved |
| `x` / `i` | Exports the configuration to a file to edit it elsewhere / imports it back (then `s` shows the changes and saves them) |
| `o` | Opens another memory card or file |
| `I` | Installs OSDHub and the programs it launches (below) |

The rest of the file is kept as it is: comments, the order of the lines, the menu entries and the settings it doesn't
know. A setting is changed in its line, or in place of its commented-out line (`# games_covers = 0`), or added at the end.

**Saving** never changes the memory card in place:

1. The memory card image (or file) is copied to `<name>.bak-<UTC date>-<time>` next to it
2. The new image is written next to it and moved over it
3. The image is read again: every file and folder in it must be readable, without clusters shared between them, and
   `OSDMENU.CNF` must be what was saved; otherwise the copy is put back

The memory card images of MMCE devices are 8 MB images without ECC; the 8.25 MB images with ECC, like PCSX2's, are
supported too, with the ECC of the pages written computed again. Edit the memory card with the SD card in a card reader,
not while the PS2 is using it. Delete the `.bak-*` copies once the new configuration works.

## Installing

`I` in the Config tab (or the `install` command) installs OSDHub and, optionally, the programs it launches, from their
latest releases on GitHub:

| | Where | From |
| --- | --- | --- |
| **OSDHub** (always) | `mc0:/BOOT/BOOT.ELF`, the ELF the boot loader starts, in the BOOT memory card image or in a folder (below) | [higorhgon/osdmenu](https://github.com/higorhgon/osdmenu/releases)'s `osdmenu-*.zip` |
| **RiptOPL** | `APPS/OPL/RIPTOPL.ELF` | [higorhgon/Open-PS2-Loader](https://github.com/higorhgon/Open-PS2-Loader/releases)'s manual releases, with the MMCE/SMB argv autolaunch OSDHub uses (`games_launcher = opl`): the OFFICIALPINNED build, or PS2DEVPINNED-RA, the RetroAchievements one |
| **Neutrino** | `APPS/neutrino/` | [rickgaiser/neutrino](https://github.com/rickgaiser/neutrino/releases)'s latest release (`.7z`) |
| **Ember** | `EMBER/`, with `EMBER/games/` | [Ember](https://github.com/Gageformer/Ember/releases), a PS1 emulator by **Gageformer**, under its beta licence (installed next to it) |

OSDHub goes in the BOOT memory card image of an MMCE device (`MemoryCards/**/BOOT/`) or, for a regular memory card, in
a folder (`OSDHUB-MC` on the device by default) holding the `BOOT` and `SYS-CONF` folders to copy to the memory card,
from a USB drive with wLaunchELF for example: `BOOT/BOOT.ELF` replaces the one on the memory card, and `SYS-CONF` can
be left out to keep the memory card's own `OSDMENU.CNF`. A folder created on the memory card gets its icon for the
PS2's Browser (OSDMenu's for `BOOT`, and the one KELFBinder installs for `SYS-CONF`), from OSDHub's sources.

Everything is downloaded first, then the files that will be written are listed (marking the ones they replace), and
they're only written after confirming. The memory card is written like the configuration is saved: copied first and
checked after. When it has no `OSDMENU.CNF`, it gets the example from OSDHub's release, which the Config tab opens.

Ember needs a PS1 BIOS dumped from your own console, which isn't (and can't be) downloaded: give it in the installer
(or with `--bios`) to copy it to `EMBER/bios.bin`. Ember's own `settings.txt` and games aren't touched.

The paths in `OSDMENU.CNF` aren't changed: the example's are `mmce?:/APPS/neutrino/neutrino.elf` and
`mmce?:/APPS/OPL/RIPTOPL.ELF`, where the installer puts them.

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

# Exports OSDMenu's configuration from the BOOT memory card, then saves it back after editing it
osdhub-manager config /run/media/$USER/MMCE --export OSDMENU.CNF
nvim OSDMENU.CNF
osdhub-manager config /run/media/$USER/MMCE --import OSDMENU.CNF

# Installs OSDHub, RiptOPL (RetroAchievements build), Neutrino and Ember, with the BIOS for Ember
osdhub-manager install /run/media/$USER/MMCE --opl-ra --neutrino --bios ~/scph1001.bin

# On a USB drive: OSDHub in USB/OSDHUB-MC, to copy to a memory card, and Neutrino on the drive
osdhub-manager install /run/media/$USER/USB --neutrino

# The configuration in a given memory card image
osdhub-manager config /run/media/$USER/MMCE/MemoryCards/BOOT/BootCard.mcd

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
| `--export FILE` / `--import FILE` | Writes the configuration to a file / saves a file as the configuration, after showing the changes and asking (`config`) |
| `--yes` | Doesn't ask before saving (`config --import`, `install`) |
| `--opl` / `--opl-ra` / `--neutrino` / `--ember` | What to install besides OSDHub (`install`); `--opl-ra` is RiptOPL's RetroAchievements build |
| `--bios FILE` | PS1 BIOS for Ember, copied to `EMBER/bios.bin` (`install`) |
| `--card FILE` | Memory card image to install OSDHub in (`install`; by default the only one in `MemoryCards/**/BOOT/`) |
| `--folder DIR` | Folder to put OSDHub's `BOOT` and `SYS-CONF` in, to copy them to a memory card (`install`; by default `OSDHUB-MC` on the device when it has no BOOT memory card image) |

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

The archives are read with [zip](https://github.com/zip-rs/zip2) and [sevenz-rust2](https://github.com/hasenbanck/sevenz-rust),
and GitHub's answers with serde_json. The dependencies are [ratatui](https://ratatui.rs), crossterm and [ratatui-image](https://github.com/ratatui/ratatui-image)
with [image](https://github.com/image-rs/image) (JPEG and PNG only) for the terminal interface, and
[ureq](https://github.com/algesten/ureq) (HTTPS with rustls), pinned by `Cargo.lock`. The release
binaries are built by GitHub Actions when a `v*` tag is pushed: a static Linux binary (musl), Windows and macOS.
