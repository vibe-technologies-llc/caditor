# Version history

Each save keeps the state it replaces inside the `.caditor` file, so earlier versions travel with
the model. {command:file.history} lists them newest first, with when each was saved and after which
change.

- **Restore** brings a version back as an ordinary change: Undo returns to what you had, and the
  next save keeps it as a version too.
- **Preview** shows what a version holds without changing the model: a picture of its bodies, how
  many features and bodies it has, any feature that fails in it, and how it differs from the
  model now (features not in the model now, added since or changed since, and parameters changed
  since). It is read and recomputed in the background; **Cancel** stops it, and **Restore this
  version** restores it from the preview. A version that cannot be read says so there.
- **Keep** marks a version so it is never thinned out; **Stop keeping** removes the mark. Kept
  versions show a bookmark and the word Kept.

## Thinning

To keep files small, older versions thin out by age: the ten newest always stay, then one an hour
for the last day, one a day for the last month, one a week for the last year and one a month
before that. Kept versions stay in every case; only a file grown past its size limit drops the
oldest ones.

Versions are only kept once the model is saved to a file. A version that cannot be read is marked
Damaged, with the reason on hover.
