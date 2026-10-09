# Saving on Windows and Microsoft Defender

Microsoft Defender scans every file a program writes. caditor writes often: each save goes through
a new copy of the file, the recovery journal is written within seconds of every change, and saves
keep the earlier versions inside the file. In a folder Defender scans, this makes saving slower and
can make the window wait on large models.

## Excluding your models' folder

You can tell Defender to leave the folder you keep your models in alone:

- Open **Windows Security** from the Start menu.
- Go to **Virus & threat protection**, then under its settings choose **Manage settings**.
- Under **Exclusions**, choose **Add or remove exclusions**, then **Add an exclusion** and
  **Folder**.
- Pick the folder holding your models.

This needs administrator rights, and an organisation may not allow it.

## What it trades away

Defender no longer scans anything in that folder. A harmful file saved there would not be caught
until it is opened or copied elsewhere. Exclude only a folder that holds your own models, never
Downloads, the desktop or a whole drive, and do not keep programs there.

caditor never changes Defender's settings itself. It reminds you of this once, the first time you
save a model on Windows; [Preferences](preferences) › General › Saving on Windows shows the
reminder again.
