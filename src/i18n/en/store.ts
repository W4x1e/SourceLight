/** English strings for the store area. See ../index.ts for the message format. */
export default {
  "store.resolve.done": { one: "Rejected {count} photo {groups}.", other: "Rejected {count} photos {groups}." },
  "store.resolve.groups": { one: "in {count} group", other: "in {count} groups" },
  "store.resolve.skipped": { one: "Left {count} already judged alone.", other: "Left {count} already judged alone." },
  "store.resolve.undo": "{key} undoes it.",
  "store.resolve.nothing": "Nothing to change.",
  "store.resolve.allJudged": "Every group already has a verdict — nothing to change.",
  "store.colours.noneClose": "No photos have similar colours, or this photo's colours haven't been read yet.",
  "store.paste.copied": "Copied {name}'s location and keywords. Paste them from the photo menu.",
  "store.paste.place": { one: "Put {name}'s place on {count} photo.", other: "Put {name}'s place on {count} photos." },
  "store.paste.positionToo": { one: "{count} with no position of its own took its position too.", other: "{count} with no position of their own took its position too." },
  "store.paste.positionOnly": { one: "Gave {count} photo {name}'s position.", other: "Gave {count} photos {name}'s position." },
  "store.paste.keywords": { one: "Put {name}'s keyword on {photos}.", other: "Put {name}'s {count} keywords on {photos}." },
  "store.track.title": { one: "Re-place {count} photo from its GPX track?", other: "Re-place {count} photos from their GPX track?" },
  "store.track.message": {
    one: "It was placed using its old time. Re-placing moves it to where the track was at the new time. Positions from the camera or set by hand don't change.",
    other: "They were placed using their old times. Re-placing moves them to where the track was at the new times. Positions from the camera or set by hand don't change.",
  },
  "store.track.confirm": "Re-place",
  "store.track.leave": "Leave them",
  "store.track.moved": { one: "Moved {count} photo along its track.", other: "Moved {count} photos along their track." },
  "store.track.outside": { one: "{count} now falls outside it and stayed where it was.", other: "{count} now fall outside it and stayed where they were." },
  "store.track.missing": { one: "The track file for {count} has gone, so it stayed put.", other: "The track file for {count} has gone, so they stayed put." },
  "store.captureTime.reverted": { one: "Put {count} photo back to the file's own time.", other: "Put {count} photos back to the file's own time." },
  "store.captureTime.nothingToRevert": "Nothing to put back — these still have their files' times.",
  "store.captureTime.fromSidecars": { one: "Took the sidecar's capture time for {count} photo.", other: "Took the sidecar's capture time for {count} photos." },
  "store.captureTime.changed": { one: "Changed the capture time of {count} photo.", other: "Changed the capture time of {count} photos." },
  "store.captureTime.undated": { one: "{count} had no time to move and was left alone.", other: "{count} had no time to move and were left alone." },
  "store.sidecarTimes.titleOne": "Use the capture time from {name}'s sidecar?",
  "store.sidecarTimes.titleMany": { one: "Use the capture time from {count} sidecar?", other: "Use the capture times from {count} sidecars?" },
  "store.sidecarTimes.oneDiffers": "Its sidecar says {sidecar}; here it's {here}.",
  "store.sidecarTimes.undated": "undated",
  "store.sidecarTimes.manyDiffer": "Their sidecars give different times from the ones here.",
  "store.sidecarTimes.why": "They were set in another app, perhaps to fix a camera's clock. The files aren't changed, and “{revert}” in {dialog} undoes it.",
  "store.sidecarTimes.useIts": "Use its time",
  "store.sidecarTimes.useTheirs": "Use their times",
  "store.sidecarTimes.keep": "Keep these times",
  "store.folderKeywords.added": { one: "Added {count} keyword from folder names to {photos}. It's under “Folders” in Keywords.", other: "Added {count} keywords from folder names to {photos}. They're under “Folders” in Keywords." },
  "store.folderKeywords.none": "No folder names to add. They're all dates, camera folders or already added.",
  "store.folderKeywords.removed": { one: "Took keywords made from folder names off {count} photo.", other: "Took keywords made from folder names off {count} photos." },
  "store.folderKeywords.noneToRemove": "No keywords from folder names to take off.",
  "store.keywords.confirmed": { one: "Confirmed {count} of the tagger's keywords.", other: "Confirmed {count} of the tagger's keywords." },
  "store.keywords.deleted": "Deleted “{name}”.",
  "store.keywords.deletedFrom": { one: "Deleted “{name}” and took it off {count} photo.", other: "Deleted “{name}” and took it off {count} photos." },
  "store.types.shapePortrait": "Portrait",
  "store.types.shapeLandscape": "Landscape",
  "store.types.shapeSquare": "Square",
  "store.types.shapePanorama": "Panorama",
  "store.types.colourOrange": "Orange",
  "store.types.colourTeal": "Teal",
  "store.types.colourPink": "Pink",
  "store.types.colourBrown": "Brown",
  "store.types.colourWhite": "White",
  "store.types.colourGrey": "Grey",
  "store.types.colourBlack": "Black",
  "store.types.faces0": "No faces",
  "store.types.faces1": "One face",
  "store.types.faces2": "Two faces",
  "store.types.faces3to5": "Three to five",
  "store.types.faces6": "Six or more",
  "store.develop.openFailed": "Couldn't open {name}: {error}",
  "store.develop.thisPhoto": "this photo",
  "store.selection.narrowed": { one: "{count} selected photo is no longer shown and was deselected.", other: "{count} selected photos are no longer shown and were deselected." },
  // Shared by more than one dialog
  "store.dialog.notNow": "Not now",

  // Offer to set up on-device AI for new photos
  "store.aiOffer.title": "Make new photos searchable?",
  "store.aiOffer.message": "SourceLight can index, tag and find people in new photos on this computer, so you can search for them by describing them. It needs a one-time download of about {size}. You can turn this off in Activity.",
  "store.aiOffer.messageMetered": "SourceLight can index, tag and find people in new photos on this computer, so you can search for them by describing them. It needs a one-time download of about {size}, and you seem to be on a metered connection. You can turn this off in Activity.",
  "store.aiOffer.confirm": "Download and analyse",
  "store.search.stillIndexing": {
    one: "Still indexing ({count} photo left). Results will improve as it finishes.",
    other: "Still indexing ({count} photos left). Results will improve as it finishes.",
  },
  "store.aiOffer.settingUp": "Setting up on-device AI ({size} download). Progress is shown in Activity.",

  // Scanning the library
  "store.scan.starting": "Starting",
  "store.scan.added": { one: "Added {count} new photo.", other: "Added {count} new photos." },
  "store.scan.relinked": {
    one: "Found {count} moved photo. Its ratings, tags and edits were kept.",
    other: "Found {count} moved photos. Their ratings, tags and edits were kept.",
  },
  "store.scan.missing": {
    one: "{count} photo can't be found. A drive may be disconnected. Its ratings, tags and edits are kept.",
    other: "{count} photos can't be found. A drive may be disconnected. Their ratings, tags and edits are kept.",
  },
  // {reason} is the first error message from the scan, as the core reported it.
  "store.scan.failed": {
    one: "{count} photo couldn't be read. {reason}",
    other: "{count} photos couldn't be read. {reason}",
  },

  // Saving a set of photos as a collection
  "store.collection.saved": {
    one: "Saved {count} photo as “{name}”.",
    other: "Saved {count} photos as “{name}”.",
  },

  // Highlights
  "store.highlights.nothing": "Nothing to choose from. Every photo here is rejected, soft or has closed eyes.",
  "store.highlights.notIndexed": "None of these photos is indexed for search yet. Index them first (Activity).",
  // The name of the collection made from the best photos of a search or a shoot.
  "store.highlights.collectionName": "Highlights · {label}",
  "store.highlights.made": "Saved the best {count} of {total} as “{name}”.",

  // Moving rejects to the Trash / Recycle Bin. {trash} is that place's name on this system.
  "store.bin.title": {
    one: "Move this reject to the {trash}?",
    other: "Move {count} rejects to the {trash}?",
  },
  "store.bin.message": "The files go to the {trash}, along with any XMP sidecars and paired JPEGs, and you can restore them from there. They're also removed from the catalogue, which is backed up first. A reject with a copy that isn't rejected is kept.",
  "store.bin.confirm": "Move to {trash}",
  "store.bin.moved": {
    one: "Moved {count} reject to the {trash} — {freed} freed.",
    other: "Moved {count} rejects to the {trash} — {freed} freed.",
  },
  "store.bin.kept": "{count} kept, because a copy isn't rejected.",
  "store.bin.skipped": "{count} skipped — the file is missing.",
  "store.bin.nothing": "Nothing was moved.",
  // Deleting chosen photos, whatever their flag. A copy is a virtual copy: a second set of edits on the same file.
  "store.delete.title": { one: "Delete this photo?", other: "Delete {count} photos?" },
  "store.delete.message": "The files go to the {trash}, along with any XMP sidecars and paired JPEGs, and you can restore them from there. They're also removed from the catalogue with any copies of them. The catalogue is backed up first.",
  "store.delete.copiesTitle": { one: "Delete this copy?", other: "Delete {count} copies?" },
  "store.delete.copiesMessage": "Only the copy and its edits go. The original and its file stay. The catalogue is backed up first.",
  "store.delete.confirm": "Delete",
  "store.delete.moved": {
    one: "Moved {count} photo to the {trash} — {freed} freed.",
    other: "Moved {count} photos to the {trash} — {freed} freed.",
  },
  "store.delete.deleted": { one: "Deleted {count} copy.", other: "Deleted {count} copies." },

  // Content search
  "store.search.indexTitle": "Index for content search?",
  "store.search.indexMessage": {
    one: "{count} photo hasn't been indexed yet, so it won't show up in content search. You can keep working while it runs.",
    other: "{count} photos haven't been indexed yet, so they won't show up in content search. You can keep working while it runs.",
  },
  "store.search.indexConfirm": "Index now",
  "store.search.indexCancel": "Search anyway",

  // Find similar
  "store.similar.label": "Similar to {label}",
  "store.similar.none": "No photos are visually similar enough to that one.",

  // Duplicates, bursts and compare
  "store.duplicates.noneLooser": "No likely duplicates found. Try a looser similarity, or index the library first.",
  "store.duplicates.noneNotIndexed": "No likely duplicates found. The library may not be indexed yet.",
  "store.stacking.none": "No bursts found. If the library isn't indexed yet, index it first.",
  "store.compare.selectTwo": "Select two or more photos to compare them.",

  // Undo. {label} says what was undone, as the core reported it.
  "store.undo.nothing": "Nothing to undo.",
  "store.copy.thePhoto": "the photo",
  "store.copy.done": "Copied {name}.",
  "store.copy.doneSized": "Copied {name}, {width} × {height}.",
  "store.copy.refused": "the clipboard refused it",
  "store.copy.failed": "Couldn't copy {name}: {why}.",
  "store.undo.done": "Undid: {label}.",
  "store.undo.beyond": "The last change ({label}) can't be undone. Undo covers ratings, flags, colour labels and pasted develop settings. Press it again to undo the change before.",

  // Library folders
  "store.folder.addTitle": "Add a folder to the library",
  "store.folder.cardTitle": "This folder is on a memory card",
  "store.folder.cardMessage": "Photos added from a card stay only on the card. If you format it, they're gone, along with your ratings and edits. Import copies them to your computer first.",
  "store.folder.cardImport": "Import from the card",
  "store.folder.cardNotNow": "Not now",
  "store.folder.cardAnywayTitle": "Add the card itself to the library?",
  "store.folder.cardAnywayMessage": "Only do this if the photos will stay on this card. SourceLight reads library folders where they are; it doesn't copy them.",
  "store.folder.cardAnyway": "Add the card",
  "store.folder.removeTitle": "Remove {name} from the library?",
  "store.folder.removeMessage": {
    one: "Its {count} photo leaves the catalogue, with their ratings, tags, places, faces and edits. The files on disk aren't touched, and the catalogue is backed up first.",
    other: "Its {count} photos leave the catalogue, with their ratings, tags, places, faces and edits. The files on disk aren't touched, and the catalogue is backed up first.",
  },
  "store.folder.removeConfirm": "Remove folder",
  "store.folder.removed": {
    one: "Removed {name} — {count} photo forgotten.",
    other: "Removed {name} — {count} photos forgotten.",
  },
  "store.folder.locateTitle": "Where is {name} now?",
  "store.folder.forgetTitle": {
    one: "Forget {count} photo outside your folders?",
    other: "Forget {count} photos outside your folders?",
  },
  "store.folder.forgetMessage": "These are from a folder that's no longer in the library. Forgetting them removes them, with their ratings, tags and edits, from the catalogue (after a backup). The files aren't touched. To keep them, add their folder back.",
  "store.folder.forgetConfirm": "Forget them",

  // Tagging
  "store.tagging.preparing": "Preparing",
  "store.tagging.retagTitle": "Re-tag every photo?",
  "store.tagging.retagMessage": "All automatic tags are replaced. Tags you added yourself are kept. The catalogue is backed up first, so you can restore it if needed.",
  "store.tagging.retagConfirm": "Re-tag everything",

  // Develop
  "store.develop.copySavedNamed": "Saved a copy: {name}. You're now editing the copy.",
  "store.develop.copySaved": "Saved a copy. You're now editing the copy.",

  // Label tables in types.ts (shown with t(key) where they are used)
  "store.types.aspectFree": "Free",
  "store.types.colourRed": "Red",
  "store.types.colourYellow": "Yellow",
  "store.types.colourGreen": "Green",
  "store.types.colourBlue": "Blue",
  "store.types.colourPurple": "Purple",
  "store.types.focalUpTo24": "Up to 24 mm",
  "store.types.focal25to50": "25–50 mm",
  "store.types.focal51to105": "51–105 mm",
  "store.types.focal106to200": "106–200 mm",
  "store.types.focalOver200": "Over 200 mm",
  "store.types.isoUpTo400": "ISO 400 and under",
  "store.types.iso401to1600": "ISO 401–1600",
  "store.types.iso1601to6400": "ISO 1601–6400",
  "store.types.isoOver6400": "Over ISO 6400",
  "store.types.apertureWider28": "ƒ/2.8 and wider",
  "store.types.aperture29to56": "ƒ/2.9–5.6",
  "store.types.apertureNarrower56": "Narrower than ƒ/5.6",
  "store.types.sortNewest": "Newest first",
  "store.types.sortOldest": "Oldest first",
  "store.types.sortAdded": "Recently added",
  "store.types.sortRating": "Highest rated",
  "store.types.sortName": "File name",
  "store.types.sortSharpest": "Sharpest",
  "store.types.sortCustom": "Custom order",
} as const;
