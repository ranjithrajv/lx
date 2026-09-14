# Host detection & smart defaults

Part of the [lx docs](../README.md). See also `lx info` in
[commands.md](commands.md).

`lx` detects the machine it's running on and adapts, instead of making you
configure per distro. `lx info` prints the whole picture — read-only and
offline, nothing downloaded or installed:

```
$ lx info
Host information
  OS               Debian GNU/Linux 13 (trixie)
  OS ID            debian 13
  Codename         trixie
  Kernel           6.12.6-amd64
  Machine          x86_64
  Architecture     amd64
  Package manager  apt
  Package format   deb
  Asset dist       trixie
```

Anything that can vary uses that detection as its **default**, so the same
command line adapts across Debian, Fedora, and Arch hosts:

| Detected | From | Where it's used |
|---|---|---|
| Native package format (`deb`/`rpm`/`arch`/`apk`/`ipk`) | the host package manager | `lx repo --format` and `lx init`'s package format (deb/rpm/arch/apk); `lx convert --to` and the consumer `install`/`upgrade`/`remove`/`list` (deb/rpm/arch) |
| Host package manager (`apt`/`dnf`/`zypper`/`pacman`/`apk`/`xbps`) | `PATH` probe | `--install-build-deps` installs missing build dependencies with the host's own manager |
| Architecture, in Debian naming | `uname -m` | `lx build --host`, `lx deps scan`'s host-only default scan |
| Distro codename / RPM family | `/etc/os-release` | the release-asset "dist" token `lx install`/`upgrade` match prebuilt packages against |

Every inferred default is announced and every flag still overrides it (`lx
convert --to rpm`, `lx repo --format deb`, `lx build --format rpm`, …), and
`lx info --json` exposes the same detection for scripts and CI.

