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
The games are shown in a table with their title ID, whether their case cover (`COV`) and disc (`ICO`) are in `ART/`,
and whether the PS2 ISOs have OPL's name (`✓`, `rename`, or `folder` for ISOs in their own subfolder):

```
┌ Games (5) ──────────────────────────────────────────────────────┐
│      Title ID    COV ICO OPL    Name                             │
│● PS2 SLUS_202.12 ✓   ✗   rename Bloody Roar 3                    │
│  PS2 SCUS_973.28 ✓   ✓   ✓      Gran Turismo 4                   │
│  PS1 SCUS_949.00 ✗   ✗   -      Crash Bandicoot (USA)            │
└─────────────────────────────────────────────────────────────────┘
```

| Key | |
| --- | --- |
| `↑` `↓` `PgUp` `PgDn` | Move |
| `Space` / `a` | Mark the game / mark all (the actions apply to the marked games, or to all the games shown) |
| `Tab` | All games, PS2 only or PS1 only |
| `c` | Downloads the art of the games, in the background, with the progress in the log |
| `t` / `f` | Art types to download (COV+ICO, COV, ICO) / downloads them again even when they're in `ART/` |
| `r` | Renames the PS2 ISOs to OPL's names, after confirming |
| `s` | Reads the games again |
| `q` | Quits |

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

# Shows how the PS2 ISOs would be renamed, then renames them
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

1. **OPL Manager's art database**, which OPL Manager downloaded from its own server until it was shut down; its backups
   are on archive.org (`PS1/<ID>/<ID>_COV.jpg`, `PS2/<ID>/...`), with every OPL art type, including the discs
2. **xlenore's [PS1](https://github.com/xlenore/psx-covers) and [PS2](https://github.com/xlenore/ps2-covers) cover
   collections** (the ones DuckStation and PCSX2 use), for the case covers

Images already in `ART/` (as `.jpg` or `.png`) aren't downloaded again unless `--force` is used.

## Building

```sh
cargo build --release
```

The dependencies are [ratatui](https://ratatui.rs) and crossterm (the terminal interface) and
[ureq](https://github.com/algesten/ureq) (HTTPS with rustls), pinned by `Cargo.lock`. The release
binaries are built by GitHub Actions when a `v*` tag is pushed: a static Linux binary (musl), Windows and macOS.
