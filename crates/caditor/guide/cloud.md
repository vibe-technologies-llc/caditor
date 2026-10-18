# Home and cloud projects

## Home

{command:file.home} opens Home, a page over the 3D view that gathers where to begin. It opens by
itself when caditor starts with an empty model, and from the File menu and the command palette at
any time. From top to bottom it holds:

- the ways to start: a sketch, a box, an import, or opening a model;
- your recent files, newest first;
- your Fusion 360 and Onshape projects, when cloud services are on;
- the samples, to see how a model is built.

A click on any of them does it and takes Home away; so does Escape, its close button, or changing
the model underneath it. Preferences › General › Starting and recent files decides whether Home
opens at startup, and a button at the foot of Home turns that off.

## Fusion 360 projects

Home advertises the Fusion 360 import until you sign in. {command:file.cloud_sign_in} opens
Autodesk's own sign-in page in your browser; caditor never sees your password, only the permission
you give it. If no browser opens, Home and the Cloud projects window offer the sign-in link to copy.

Once signed in, the line under your recent files is one line per project of your Fusion hub, with
its newest designs beside it. **One click on a design imports it**: Autodesk converts it to a STEP
file, caditor downloads that, and it comes in as an [imported body](imported-bodies) like any STEP
file, with the usual report. A design shows Autodesk's picture of it on its tile once that arrives
(a design Autodesk has not rendered keeps the icon); the pictures are kept on this computer and
removed when you sign out. The progress and a Cancel button show where the designs are, and in the
status bar once the file is being read. If a conversion fails, the page says why and offers Try
again.

Fusion's own file format is closed, so a design arrives as it is converted: a body you can
measure, cut, combine and build on, not a Fusion feature history. Each imported design is kept as a
STEP file so Reload import from its file works; the Cloud projects window shows how many are kept
and clears them.

If a team hub says Autodesk did not allow caditor, its administrator may need to allow the app in
the hub's settings. Your own personal hub needs nothing.

When you have more than one hub, the one you pick in the Cloud projects window is the one Home lists
from then on, also after restarting caditor; if it is gone, Home goes back to the team hub. Signing
out forgets the choice.

## Onshape documents

Home shows Onshape below Fusion 360. {command:file.onshape_sign_in} opens Onshape's own sign-in
page in your browser, and the line under your recent files is then one line per document, with its
Part Studios and assemblies as tiles. **One click imports one**: Onshape exports it to a STEP file,
caditor downloads it, and it comes in as an [imported body](imported-bodies) like any STEP file.
caditor asks Onshape to export only what you click and never changes your documents; signing out
only forgets the sign-in on this computer, and you end the permission itself in your Onshape account.

## The Cloud projects window

{command:file.cloud} (File › Cloud) opens the window for everything that does not fit on Home:

- **Accounts**: whether you are signed in, Sign in, Cancel while the browser is awaited, Sign out
  and Refresh.
- **Projects**: for each signed-in service, the hub to list (Fusion 360: the team hub first), a list
  of its projects or documents and, beside it, the folders and designs of the one you choose, with
  a breadcrumb to go back up and an Import button on each design.
- **Downloaded designs**: how many STEP files are kept, how large they are, and Clear.
- **Cloud features**: whether projects show on Home, and Turn off cloud features.

Every command has a name in the command palette: Home, Cloud projects, Sign in to Fusion 360, Sign
out of Fusion 360, Sign in to Onshape, Sign out of Onshape and Refresh cloud projects.

## Turning it off

Cloud services are optional. **Turn off cloud features** on Home, in the Cloud projects window, or
under Preferences › General › Cloud hides every cloud button, makes no request to any cloud service,
signs you out (revoking the sign-in where the service allows it) and forgets it. The same switch turns it back on,
and Restore defaults never does. While you are signed out, caditor contacts Autodesk only when you
press Sign in.

## When it does not work

- *This build has no Fusion 360 app registered* (or no Onshape app). Sign in needs an app registered
  with the service; builds from releases that carry one work at once, and you can type your own
  app's client ID under Preferences › General › Cloud.
- *caditor could not listen for the browser.* Another program holds the local port the sign-in
  returns to; close it and try again.
- *Autodesk is taking too long.* A very large design can take several minutes to convert; Try
  again picks up where Autodesk got to.
