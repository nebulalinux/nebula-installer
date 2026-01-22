<p align="center">
	<a href="https://github.com/nebulalinux"><img src="https://i.imgur.com/4PjBVpt.png" style="border-radius: 50%;" height="200" width="200" alt="Nubela Linux"></a>
</p>

<h4 align="center">TUI installer for <a href="https://github.com/nebulalinux">Nebula Linux</a></h4>

![nebula-installer screenshot](https://i.imgur.com/MhuQo6t.png)

| Name 1 | Name 2 | Name 3 |
| --- | --- | --- |
| ![Screenshot 1](https://i.imgur.com/i3bYHpt.png) | ![Screenshot 2](https://i.imgur.com/41opvUy.png) | ![Screenshot 3](https://i.imgur.com/a08KXY7.png) |

| Name 4 | Name 5 | Name 6 |
| --- | --- | --- |
| ![Screenshot 4](https://i.imgur.com/mTa5TRV.png) | ![Screenshot 5](https://i.imgur.com/4knGrF1.png) | ![Screenshot 6](https://i.imgur.com/Np8IIHy.png) |

### Build (local)

```sh
cargo build --manifest-path nebula-installer/Cargo.toml
```

### Run (local)

```sh
cargo run --manifest-path nebula-installer/Cargo.toml
```

See the Env Vars section below for local overrides

### Env Vars (local dev)

Copy `.env.example` to `.env` in the repo root and edit as needed. The installer loads it on startup

| Variable | Default | Purpose |
| --- | --- | --- |
| `NEBULA_SKIP_NETWORK` | `0` | Skip the network step when set to `1` |
| `NEBULA_OFFLINE_ONLY` | `0` | Force offline-only install when set to `1` |
| `NEBULA_DEV_GPU` | empty | Override GPU detection (comma-separated, e.g. `nvidia,intel,amd`) |
| `NEBULA_DEV_ALLOW_NONROOT` | `0` | Allow running the installer without root when set to `1` |
| `NEBULA_OUTER_GAP` | `24` | Adjusts terminal wrapper outer gap used by live scripts |
| `NEBULA_SKIP_OFFLINE_REPO` | `0` | Skip building the ISO offline repo when set to `1` |
| `NEBULA_PACMAN_MIRROR` | empty | Base URL for pacman mirrors (e.g. `https://mirror.nebulalinux.com/stable`) |
| `NEBULA_PACMAN_MIRRORLIST` | empty | Full mirrorlist contents, overrides `NEBULA_PACMAN_MIRROR` when set |

### Config

The installer reads `nebula-installer/config.toml` at build time (embedded into the binary).
Use it to manage:
- Base package lists (`[packages]`)
- App selection lists (`[selections]` for browsers, editors, terminals, compositors)

### Live Installer

- Select target disk
- Provide keyboard layout, timezone, hostname, user, and passwords, etc
- Installer configures LUKS + Btrfs + GRUB (UEFI/BIOS). Currently supports only Btrfs
- Installer runs inside Kitty terminal on Labwc (Wayland)
- Wallpaper: `nebula-iso/airootfs/usr/share/backgrounds/nebula/1.jpg`
- Boot splash theme: `nebula-iso/airootfs/usr/share/plymouth/themes/nebula-splash`
- GRUB theme: `nebula-iso/grub/themes/nebula-vimix-grub`
- Pacman mirrors: `nebula-iso/airootfs/etc/pacman.d/mirrorlist`
- Offline repo: `nebula-iso/airootfs/opt/nebula-repo` is configured in `nebula-iso/airootfs/etc/pacman.conf` and is preferred during install when present
- Offline repo key: place `nebula-repo.gpg` at repo root or in `nebula-iso/airootfs/opt/nebula-repo` to bundle it into the ISO
- Offline-only mode: `NEBULA_OFFLINE_ONLY=1` forces install to use only `nebula-offline` and fail if anything is missing

### Dev Run Notes

- `sudo -E NEBULA_SKIP_NETWORK=1 ./nebula` bypasses the Network step and continues.
- `sudo NEBULA_DEV_GPU=nvidia,intel ./nebula` overrides GPU detection for dev runs.

or use env variables
