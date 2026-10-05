# Installing caditor on Windows

caditor runs on 64-bit Windows 10 (version 1809 or later) and Windows 11, with a graphics driver
offering Direct3D 12, Vulkan or OpenGL.

## Install

Open `caditor-<version>-windows-x86_64.msi`. It installs caditor for your account only, without
asking for administrator rights:

- the program in `%LOCALAPPDATA%\Programs\caditor`,
- a caditor entry in the Start menu,
- `.caditor` models opening in caditor from File Explorer.

The installer is not code-signed, so Microsoft Defender SmartScreen may say it does not recognise
it; choose More info, then Run anyway. To install without any window, for example from a script:

```powershell
msiexec /i caditor-<version>-windows-x86_64.msi /qn
```

Installing a newer version replaces the older one.

## When caditor does not start

If caditor cannot open its window, it says why in a message. When the graphics driver is at
fault, install the latest one from the maker of the graphics card. If that does not help, add the
user environment variable `WGPU_BACKEND` with the value `vulkan` (or `gl`) in Settings, under
System, About, Advanced system settings, Environment Variables, and start caditor again: it then
draws through Vulkan (or OpenGL) instead of Direct3D 12.

Each run keeps a log in `%LOCALAPPDATA%\caditor\logs`; the ten newest are kept. After caditor
stops unexpectedly, it names the log of that run when it next starts.

## Uninstall

Uninstall caditor from Settings, Apps, Installed apps. Your models, preferences
(`%APPDATA%\caditor`) and crash-recovery journals (`%LOCALAPPDATA%\caditor`) are left where they
are.

## Verify the download

Next to each installer is a `.sha256` file. In PowerShell, compare its first word with:

```powershell
(Get-FileHash caditor-<version>-windows-x86_64.msi).Hash.ToLower()
```

Each installer also carries a build provenance attestation, signed through Sigstore by the release
workflow that built it. With the GitHub CLI, check that it came from that workflow:

```powershell
gh attestation verify caditor-<version>-windows-x86_64.msi --repo <owner>/<repository>
```

naming the repository the release was downloaded from.

## Licence

caditor is licensed under the GNU Affero General Public License, version 3 only
(`licenses\LICENSE.txt` in the installation folder). The source code of each release is published
with it. The licences of the libraries and the font built into caditor are in
`licenses\THIRD-PARTY-LICENSES.html` and `licenses\Inter-LICENSE.txt`.
