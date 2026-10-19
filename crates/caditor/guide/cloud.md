# Home and cloud projects

## Home

{command:file.home} opens Home, a page over the 3D view that gathers where to begin. It opens by
itself when caditor starts with an empty model, and from the File menu and the command palette at
any time. From top to bottom it holds:

- the ways to start: a sketch, a box, an import, or opening a model;
- your recent files, newest first, each with a small picture of its bodies when the model was
  saved by this version of caditor (older files and models without bodies keep a file icon until
  they are saved again);
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

Other files in a project folder, uploaded as they are, are not converted: the Cloud projects window
lists the STEP, STL, OBJ, 3MF, DXF and SVG files of a Fusion 360 folder beside its designs, and Import
downloads one and brings it in exactly as if you had opened it from your disk, with its usual
report or questions (a mesh asks for its unit, a drawing for where to place it).

Fusion's own file format is closed, so a design arrives as it is converted: a body you can
measure, cut, combine and build on, not a Fusion feature history. Each imported design is kept as a
STEP file so Reload import from its file works; the Cloud projects window shows how many are kept
and clears them.

If a team hub says Autodesk did not allow caditor, its administrator may need to allow the app in
the hub's settings. Your own personal hub needs nothing.

When you have more than one hub, the one you pick in the Cloud projects window is the one Home lists
from then on, also after restarting caditor; if it is gone, Home goes back to the team hub. Signing
out forgets the choice.

## Sending the model to Fusion 360 or Onshape

{command:file.cloud_send} (File › Cloud, and the command palette once you are signed in) opens the
Cloud projects window to choose where the model goes. Open one of your projects, open the folder it
belongs in and press **Send the model here**. caditor exports the bodies chosen in the Export dialog
as a STEP file named after the model, without asking for a file, and uploads it to that folder; it
never changes any other file in the project. The progress and a Cancel button show at the head of
the Fusion 360 section, and a notice says when the file is in.

If the folder already holds a file of that name, the upload becomes its **new version**, so
Fusion's own version history keeps the earlier ones; otherwise it is a new file. Fusion 360 keeps
the STEP file as it is, as an uploaded file: the Cloud projects window lists it beside the designs
and Import brings it back. The top of a project that has several folders cannot hold files, so
Send is offered once you have opened a folder.

Nothing is sent if the export leaves a body out (the notice says which and why), if the model is
still recomputing, or if there is no body yet. If Fusion 360 refuses (you may only be allowed to
view that folder), the window says so and offers Try again. A sign-in made before sending existed
needs signing out and in once more, because sending asks Autodesk for one more permission.

Onshape receives the model the same way: in the Cloud projects window open a document, then its
main workspace (the first thing listed) or one of its branches, and press **Send the model here**.
The STEP file is added to the document as a file element named after the model, and Import beside
it brings it back. If the document already holds a file element of that name, it is updated in place
and Onshape keeps the earlier contents in the document's history. Beside it, **Send as a Part
Studio** has Onshape import the same STEP file as a new Part Studio in the workspace instead; the
file is then not kept as an element, and a second send makes a second Part Studio. The progress
shows as the file goes up and then while Onshape turns it into a Part Studio, which can take a
while; Cancel stops the upload at once and, once the file is up, stops waiting, though Onshape may
still finish adding the Part Studio. A saved version of a document cannot be changed, so Send is
disabled there. If Onshape refuses (you may only be allowed to view the document), or cannot turn
the file into a Part Studio, the window says so and offers Try again. Files you uploaded show the
date they were last changed when Onshape's history of the workspace tells it; a file it cannot date
shows none.

## Onshape documents

Home shows Onshape below Fusion 360. {command:file.onshape_sign_in} opens Onshape's own sign-in
page in your browser, and the line under your recent files is then one line per document, with its
Part Studios and assemblies as tiles. **One click imports one**: Onshape exports it to a STEP file,
caditor downloads it, and it comes in as an [imported body](imported-bodies) like any STEP file.
The **View** list in the Cloud projects window chooses which documents are listed: yours, the ones
you opened recently, the ones shared with you, those of your companies and teams, and public ones.
In that window a document's other branches and its versions appear as folders beside the tiles of
its main workspace, so you can import a Part Studio as it was at a version. caditor asks Onshape
to export only what you click and never changes your documents; signing out only forgets the
sign-in on this computer, and you end the permission itself in your Onshape account. Files you
have uploaded to a document (STEP, mesh and drawing files) are listed beside its Part Studios, and
**Send the model here** puts the model into a document, as described under Sending the model above.

## The Cloud projects window

{command:file.cloud} (File › Cloud) opens the window for everything that does not fit on Home:

- **Accounts**: whether you are signed in, Sign in, Cancel while the browser is awaited, Sign out
  and Refresh.
- **Projects**: for each signed-in service, the hub to list (Fusion 360: the team hub first), a list
  of its projects or documents and, beside it, the folders and designs of the one you choose, with
  a breadcrumb to go back up and an Import button on each design and on each STEP, mesh or drawing
  file, **Send the model here** inside a Fusion 360 folder or an Onshape workspace, and **Send as a Part Studio** beside it in an Onshape workspace. **Versions** under a Fusion 360 design lists its saved versions, newest first;
  Import beside one has Autodesk convert that version instead of the newest, and it is kept as
  its own file named for the version.
- **Downloaded designs**: how many downloaded files are kept, how large they are, and Clear.
- **Cloud features**: whether projects show on Home, and Turn off cloud features.

Every command has a name in the command palette: Home, Cloud projects, Sign in to Fusion 360, Sign
out of Fusion 360, Sign in to Onshape, Sign out of Onshape, Refresh cloud projects and Send model to
the cloud.

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
