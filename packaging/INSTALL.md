# Installing caditor

caditor runs on 64-bit Linux with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and
anything later) and a GPU driver with Vulkan or OpenGL ES 3. It works on Wayland and X11.

## Install for yourself

```sh
./install.sh
```

This copies the program to `~/.local/bin/caditor` and adds its menu entry, icon and the
`.caditor` file type to `~/.local/share`, so models open from the file manager.

## Install for every user

```sh
sudo ./install.sh --prefix /usr/local
```

## Run without installing

```sh
./bin/caditor
```

## When caditor does not start

If caditor cannot open its window, it says why in a dialog (or on the terminal it was started
from). When the graphics driver is at fault, starting it with OpenGL instead of Vulkan often
helps:

```sh
WGPU_BACKEND=gl caditor
```

Each run keeps a log in `~/.local/state/caditor/logs/` (or `$XDG_STATE_HOME/caditor/logs/`); the
ten newest are kept. After caditor stops unexpectedly, it names the log of that run when it next
starts.

## Uninstall

Run the same command with `--uninstall`, for example `./install.sh --uninstall`. Your models,
preferences and crash-recovery journals are left where they are.

## Verify the download

Next to each archive is a `.sha256` file:

```sh
sha256sum -c caditor-*.tar.zst.sha256
```

Each archive also carries a build provenance attestation, signed through Sigstore by the release
workflow that built it. With the GitHub CLI, check that the archive came from that workflow:

```sh
gh attestation verify caditor-*.tar.zst --repo <owner>/<repository>
```

naming the repository the release was downloaded from.

## Licence

caditor is licensed under the GNU Affero General Public License, version 3 only
(`share/licenses/caditor/LICENSE`). The source code of each release is published with it.
The licence of the font built into caditor is in `share/licenses/caditor/Inter-LICENSE.txt`.
