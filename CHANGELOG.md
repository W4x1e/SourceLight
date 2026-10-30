# Changelog

What changed in each release of SourceLight, written for the people using it.
Beta testers read this to learn what a new build fixes and what to try — it is
the cheapest trust signal there is, so every user-visible change gets a line.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
versions follow [Semantic Versioning](https://semver.org/).

**How it is used.** Changes land under **Unreleased** as they merge. To release,
rename that heading to the version and date — `## [0.1.0] - 2026-10-01` — and run
`./scripts/release.sh 0.1.0`, which takes the release notes from that section
and refuses to draft a release without one.

## [Unreleased]

### Changed
- **SourceLight is now open source**, under the Mozilla Public License 2.0.
  Anyone may use it, read its code, change it and pass it on; changes to
  SourceLight's own files that you distribute are shared under the same
  licence. The licence key and the trial that were planned are gone, so
  nothing is locked or ever will be. There is no agreement to accept on first
  launch any more: Settings → About → Licences shows the licence and the
  third-party notices.
- **Select subject is about fourteen times faster on a graphics card**, and
  uses a fraction of the memory: 1.5 s down to about 0.1 s on an RTX 5070 Ti.
  PCs whose graphics run it download a lighter version of its model
  (115 MB instead of 224), the next time it's used.
- **Select subject and Click to select prepare the photo ten times faster.**
- **Analysing a library leaves the rest of the computer alone.** The AI
  models no longer keep processor cores busy while they wait for work, and
  automatic analysis runs below normal priority, so the window stays
  responsive while a big library is analysed on a slower PC.
- **Analysing on a PC without usable graphics is faster:** the search index
  runs about 1.5 times as fast there, in less memory, and the face pass's
  working memory is freed when it finishes. The vision model for Detailed
  descriptions recommends its smallest size on such a PC.
- **Model downloads pick up where they stopped** when a slow or patchy
  connection drops part-way, instead of starting again from nothing.
- **Setting a place or adding a keyword for many photos says how far it
  reached** — "Put “Karoo” on 40 photos." — since the selection panel only
  shows one of them, and a change that fails now says why instead of
  failing silently, keeping what you typed.
- **The welcome screen shows SourceLight's own mark**, the lit grid from the
  app icon and the mode rail, in place of a sparkle, and its two ways in are
  the same width.
- **A mask's sliders read like the rest of Develop**: they sit under an
  "Adjust this mask" heading, in the same order as the whole-photo ones
  (white balance, then light, then presence), and the five new-mask buttons
  fill two even rows instead of leaving "+ Colour" on a row of its own.
- **Export is the one highlighted button in Develop.** Done in Crop, Masks
  and Retouch is a plain button now, and Retouch's switches (Spots or Erase,
  Heal or Clone, Click or Brush, Add or Take away) look like every other
  switch in the panel.
- Keywords are called keywords in the Lightbox and the selection panel's
  box for adding them, as in the filter rail, where they were "tags" in
  English, German and Japanese. Insights' Busiest days says what its two
  columns are, as Cameras and Lenses do.
- Colour labels are called that in the filter rail and in Cull, as they are
  in the selection panel; "Colour" alone sat beside Palette, the colours in
  the photo.
- Develop spells colour the way the rest of SourceLight does: **Colour
  mixer**, **Colour grading** and the **Colour** noise slider.
- Develop's Fit, 1:1, zoom and Reset all buttons name their keyboard
  shortcuts in their tooltips (Z, = and −, Ctrl+Shift+R), as Undo, Crop and
  Mask already did.
- Settings › AI › Storage names each model after the feature it serves, in
  your language ("Select subject", "Search and tagging"), and every setting
  on the tab has its name in the same size and weight. What each model is
  for is said in full, where it was cut off mid-sentence.
- Settings › Phone: "Away from home" sits in a card like the switch above
  it, and Pair a phone is a plain button, leaving Done the one highlighted
  button in Settings.
- **A large library stays quick.** Going back to Browse, folding the filter
  rail and scrolling the grid fast could hold the window up for seconds in a
  library of tens of thousands of photos, while every date heading was
  written out again. Each is now written once.
- **Things move as they come and go.** Messages rise in and fade out, dialogs
  fade away when closed instead of vanishing, menus open from the corner
  they hang from, the Lightbox, Compare and the slideshow fade up, and
  switching workspace fades the new one in. Reduce motion in your system
  settings turns it off.

### Added
- **Activity says how long analysis has left.** Indexing, faces, tagging and
  scans show "About 12 min left" once they have a pace, measured on your
  computer: the same library takes minutes on one PC and hours on another.
- **Escape clears the selection in Browse**, as it closes everything else.
  It can be rebound under Keyboard shortcuts, and the selection panel's ×
  says so. The date picker and the Add to collection list close on Escape
  first, and only then does the next Escape clear the selection.
- **The Export dialog says what it will write** before you export: the size
  in pixels (or the long edge, for several photos), the file type and quality
  or bit depth, and the colour space — "2048 × 1365 px · JPEG, quality 92 ·
  sRGB".
- **Show in Explorer (Finder on a Mac)** on the toast after exporting one
  photo, to go straight to the file. The export queue's way to a finished
  batch's folder says the same, where it was a bare folder icon.
- **Shift-drag a slider for fine control** in Develop and Export: the value
  moves a tenth as far as the pointer, from where it was. Shift with the arrow
  keys takes ten steps on a focused slider, as it already did in its number box.
- **The colour grading wheels reset on a double-click**, as every slider
  does, and can be used from the keyboard: Tab to a wheel, then ←/→ turn
  the hue and ↑/↓ add or take away colour (Shift for bigger steps).

### Fixed
- Develop's before and after can be compared zoomed in: press and hold the
  photo still to see the before, and Split zooms and pans like the other views.
- Undo and Redo (Ctrl+Z, Ctrl+Shift+Z) take back and restore the last click
  of Click to select, instead of leaving the click behind.
- Click to select carries on from a selection's clicks when you pick it again,
  after leaving it or the mask tool, rather than starting over.
- Magic erase on several people at once, far apart in the frame, left soft
  fills. Each one is now filled on its own, as sharp as a single erase.
- Film grain looked like a fine grid of little bricks, most of all up close.
  It's now soft, clumped grain like film's, at the same strength.
- A message already showing when Import, Export or Insights opened sat on
  that dialog's buttons in a small window. It moves to the top now.
- The first key pressed straight after answering a dialog or closing
  Settings could go unheard.
- With Reduce motion on, the progress bar of a single-photo export stood
  still, as if stuck. It keeps moving, slowly.
- Places cut its hint off mid-sentence in a narrow window; it now shows
  whole or not at all.
- **Magic erase works on Windows PCs with a graphics card.** It failed at
  its first window there; it now runs on the processor, as on a Mac.
- **Select subject works on integrated graphics**, where it ran out of
  memory and failed every time. If a graphics card still can't run it, it
  now carries on with the processor instead of failing.
- **Automatic analysis pauses while you're in Develop**, as it said it did,
  instead of carrying on with a pass it had already started.
- **Messages about something that failed no longer start with "Error:"** —
  "That didn't work: Error: …" in toasts, under AI buttons and in Settings
  across the app. They say what happened, in your language, and stop.
- **The library menu fits in every language.** "Rescan all" ran out of the
  side of the menu in German, French, Spanish and Japanese, and a folder's
  Remove… sat alone on a second line in English.
- **The right-click menu on a photo is in your language throughout.** Eight
  of its items were always in English, as were the messages after changing a
  capture time, taking times from sidecars and resolving duplicates. Its stars
  now show the rating the photos have rather than five empty ones, and with
  two or more selected it offers Compare, as the selection panel does.
- **Tab stays inside the Lightbox, Compare and the slideshow**, instead of
  wandering into the library hidden behind them, and closing one puts focus
  back on the photo you opened it from, so the arrow keys carry on in the
  grid. The Lightbox's filmstrip is one stop on Tab, not one per photo.
- **Duplicate review's buttons stay on screen at the smallest window.**
  Keep this one and Close ran off its right edge (in German, most of the
  buttons did); below 1280px the secondary actions show as icons, as in
  Cull.
- **An import's report shows why each file wasn't copied in full**, such as
  "the card or its reader may be failing"; it was cut to one line, and the
  tooltip held only the file's path.
- **Adding your first folder no longer says "No photos were found"** while
  its photos are still being read. The grid says they are being read, with
  how far it has got, until the first of them appear, and the status bar no
  longer reads "Scanning 0/0" while the folders are walked. A library with
  no photos in it offers Add folder… as well as Scan.
- **The selection panel's buttons stay inside it in French and German**,
  each taking a row of its own when the two don't fit side by side.
- **Counts line up down the filter rail and the Places list.** Keywords,
  collections, collection groups, shoots and places held their counts in
  from the rest to leave room for buttons that only show on hover; the
  buttons now appear over the count instead. Their counts are formatted for
  your language, as the others are.
- **Capture times read day first in English**, "3 Jan 2025, 10:02:00", in
  the Edit capture time dialog, as dates do everywhere else in the library;
  it said "Jan 3, 2025, 10:02:00 AM". A shoot's "imported on" says the day
  as the timeline does, not "9/20/2026".
- **The arrow keys step through photos in Develop when it was opened with D**
  from the library. They moved the library's selection behind Develop
  instead.
- **Stepping to the next photo in Develop shows that it is opening**: the
  filmstrip lights the new frame at once and the photo says "Opening …"
  until it arrives, rather than the last photo sitting there unchanged.
- **Tab reaches Develop's sliders straight after the filmstrip.** Every
  frame in the filmstrip was a stop of its own, so the editing panel was
  behind as many Tab presses as the shoot had photos; now only the frame
  being edited is, and the arrow keys step from it.
- **Develop's History list works from the keyboard**: it opens on the step
  the photo is at, ↑ and ↓ move through the steps and Enter goes to one. The
  arrows changed the photo under the open list.
- **Escape in a slider's number box puts the value back** instead of applying
  what you had typed, and holding an arrow key in the box makes one step in
  History rather than one for every repeat.
- **Slider values use your language's decimal mark** (0,35 in German, French
  and Spanish) like every other number in the app, and typing a comma works.
  It was silently ignored.
- **Masks and spots can be chosen from the keyboard**: Tab now reaches each
  one in the list.
- **A first AI noise reduction or erase shows one progress bar while its
  model downloads**, with one Cancel, instead of an empty "Starting…" bar
  above the download's. The note on the photo during a first Select subject
  says it is downloading, and how far, rather than "Getting ready…".
- **Enter answers the Copy settings, Copy to selected and Save preset
  dialogs** from anywhere in them, not only from the preset name box, and
  their All/None switch is a button beside Cancel ("Tick all", "Clear all").
- **Settings stays the same size as you switch tabs**, so the tabs no longer
  jump out from under the pointer and Done stays put.
- A photo's genre and tags are capitalised in the Lightbox and the
  selection panel as they are everywhere else ("Portrait", not "portrait").
- A long place, shoot or collection name cut short in the filter rail shows
  in full on hover, as do the Places header and a trip's details at the
  smallest window, and the library's date span no longer breaks inside a
  date.
- With a keyword's menu, the library menu, the date picker or the Add to
  collection list open, the rating and flag keys no longer change the
  selected photos behind it.
- People's ⋯ menu sets its two destructive actions apart, and works from
  the keyboard like the photo menu: focus goes to its first item, and the
  arrow keys move between them.
- The timeline shows the focus ring when Tab reaches it, and Cull's
  suggested-cull note shows in full on hover when the window is narrow.
- Hovering the timeline no longer switches its line of text into the
  monospaced face: only the count is set as a readout, as it is in People's
  photo counts and the tour's step count.
- The toast after exporting one photo on Windows named its whole path rather
  than the file.
- The edge of Develop's editing panel goes back to its default width on a
  double-click, as the Presets panel's does.
- In the Presets panel, Escape clears the search box, and "For this photo"
  says when there is nothing to suggest instead of showing an empty heading.
- The dot or count that says a tool holds edits sits beside that tool's icon.
  The crop's dot was in the corner against the Mask button.
- The histogram's clipping readouts stay on one line in every language: in
  Japanese or French on a narrow panel, each one wrapped, leaving its number
  or its dot alone on a second line.
- Smaller things: "Clear" beside the colour labels is capitalised like
  every other button; Cull's Auto-advance arrow points forward; Duplicate
  review marks the sharpest frame in the pick's green rather than the amber
  of a warning; People's search bar no longer touches the cards beneath it;
  the Import dialog's counts and the search banner's count use your
  language's number format; and the top bar's size slider, date-headings
  toggle and shortcuts button are named for screen readers.

## [0.2.1] - 2026-10-02

### Added
- **Hold Space and drag to move a zoomed photo** while masking or picking a
  subject, where a plain drag paints or picks. The middle button still works.
- **A log of each session.** SourceLight now writes what it's doing to a
  file in its log folder as it goes, including which AI jobs are running on
  the graphics card. If the computer freezes or loses power while SourceLight
  is open, that session's log is kept. The next time SourceLight starts, it
  tells you and offers to open the folder. Nothing is sent anywhere.

### Changed
- **Exporting many photos is up to twice as fast.** Several are encoded at
  once instead of one at a time: twelve full-size raws went from 8 seconds
  to 3.5 on an 8-core PC.
- **Adding raws from high-resolution cameras is lighter.** Their previews
  are made in half the time, and without a 100 MB working copy for each photo
  being added.
- **Adding phone photos and colour-managed exports is about 3.5x faster.**
  An iPhone photo (Display P3) or an Adobe RGB export is shrunk to its preview
  before its colours are converted, not converted whole: 1.5 s down to 0.4 for
  a 24 MP photo.
- **Typing a search does less wasted work.** Each pause in typing
  starts a search; the ones you have typed past now stop, instead of running
  to the end beside the one you want.
- **Copy image and every export start about 0.4 s sooner**, and an export
  no longer sets up a second graphics device beside Develop's: the app
  uses one for both.

### Fixed
- **The computer could freeze** when an AI mask, AI noise reduction or face
  analysis ran while the vision model was tagging or describing photos. They
  now take turns on the graphics card: tagging pauses for a moment while the
  mask is made, then carries on.
- Taking the capture times that reading sidecars offered no longer freezes
  the window when there are thousands of them, and one bad time leaves none
  of them applied.
- **Develop no longer zooms out past fitting the photo**, which made it
  blurrier with every scroll. The filmstrip scrolls sideways with the mouse
  wheel, has no stray scrollbar, and the arrow keys change photo again.

## [0.2.0] - 2026-10-02

### Changed
- **The app is now called SourceLight.** Your library comes with it. The
  first time the new version starts, it moves your catalogue, previews,
  downloaded models and settings to SourceLight's own folder. If you have the
  earlier build installed as "Photo Organiser", you can uninstall it
  afterwards. You'll be asked to accept the licence agreement again, because
  it now carries the new name.
- **Quicker in a big library.** At 50,000 photos the counts down the side
  come back about ten times faster and typed search about twice as fast.
  Rating or keywording a photo no longer reloads the grid. The first start
  after updating spends a few seconds tidying the catalogue, which is backed
  up first.
- **Less memory.** AI noise reduction needs half the memory it did, and
  Develop's AI models and the description model are let go once you're done
  with them.
- **Sidecars are a job you can watch and stop.** Reading every sidecar asks
  first and backs up the catalogue, and writing them leaves alone the ones
  that wouldn't change.
- **Finding people asks first.** Nothing about faces is worked out until you
  say yes, and Settings can erase every face and name it found.
- **Cull** shows what it's culling, goes in capture order, and keeps your
  zoom from one frame to the next. The next frame is ready before you get
  there.
- Exports and sidecars include only what you chose.
- Plainer wording throughout.
- A new app icon.

### Added
- **Crop shows the turn, flip and straighten**, with a grid while you
  straighten. You can drag the crop's edges, not just its corners, and masks
  and spot removals stay on the subject when the crop changes.
- **See what a mask covers**, and the brush's size before it paints.
- **Export says how each file went**, and remembers your settings.
- **Merging two people with different names asks first**, and any merge can
  be undone.
- **Card import** is offered on the welcome screen. It checks the copies
  against the card itself, ends by saying whether the card is safe to
  format, copes with a camera clock that was set wrong, and remembers your
  settings.
- **More from the keyboard.** Shortcuts work on AZERTY, QWERTZ and other
  layouts. The tone curve, the timeline, Develop's zoom and the controls
  that only showed on hover can all be reached by keyboard, and screen
  readers hear which controls are on.
- High-contrast themes, and verdicts you can tell apart without colour.
- Every language has all its text, and the window fits French and German at
  its smallest size.

### Fixed
- **A downloaded Mac app is no longer called "damaged".** macOS refused to
  open it and offered only the Bin, because only part of the app was signed.
  Now it is signed as a whole, so the first launch asks the usual question
  about an app from outside the App Store, and Privacy & Security's
  Open Anyway lets it in.
- **200-megapixel photos** can be added, and photos wider than 8,192 pixels
  open in Develop instead of crashing it.
- One malformed file no longer crashes the app, or the scan, on every start.
- Removing a folder no longer freezes the window.
- On Windows, Develop's preview follows a slider while you drag it, not only
  when you let go.
- Files the app writes are written whole or not at all, and never over a
  photo.
- Undo covers copying settings to many photos, and Develop keeps its history.
- Moved, unplugged and relocated photos keep their history.
- Trip names, sidecar keywords and deleted looks were silently overwritten.
- Enter on a focused Cancel button cancels, Cancel on Save as copy cancels,
  and Escape closes only the innermost thing.
- A broken edit can no longer break Develop for good, and the crash screen
  has a way back.
- Photos are binned and de-duplicated by file, not by name.
- Click to select works after a crop.
- Search keeps working after the trial, and tagging works on a fresh
  install.

## [0.1.0] - 2026-09-30

### Added
- **Detailed descriptions are two to six times quicker, and a far smaller
  download.** The vision model now runs on llama.cpp's own server, started
  by the app, instead of through Ollama: on a fast graphics card a photo is
  tagged in about a quarter of a second rather than 0.7, and on a PC without
  one in about 5 seconds rather than 28. The download for the runtime is
  33 MB instead of 1.4 GB, and works on NVIDIA, AMD and Intel graphics alike.
  The model runs on the same graphics chip as the rest of the AI, or on the
  processor when there is only integrated graphics. The old runtime and
  its models are removed from the app's folder the first time it starts, and
  the model downloads again once, from Qwen directly.
- **Descriptions no longer run on and on.** The small vision model could
  repeat the same sentences until it hit its limit; it no longer does. And
  tags always come back complete, where a small model could before get stuck
  listing one word and leave the photo untagged.
- **Develop follows the slider as you drag, on a Mac.** The photo is drawn
  straight onto the graphics chip instead of being sent to the window as an
  image, so exposure, colour and every other slider show their effect while
  you move them, not when you let go — the split view and the histogram
  included — and zooming and panning keep up too.
  If the preview ever shows blank or out of place, Settings › About ›
  Diagnostics can switch it back.
- **Descriptions write themselves.** With Detailed descriptions on, the
  library is described in the background, a few photos at a time, after the
  quicker passes — so a new import is searchable and tagged within minutes
  even while a large library is still being described. Turning Detailed
  descriptions on starts it.
- **Settings › AI › Automatic** says what runs by itself and lets you switch
  each pass off: indexing for search, tags, faces and people, and
  descriptions — or all of them at once, as the switch in Activity does.
- **Waits for the charger.** On a laptop running on its battery, tags and
  descriptions from the vision model wait until it is plugged in, then carry
  on by themselves; indexing, faces and the built-in tagger keep going.
  Analyse now runs everything anyway, and Settings › AI › Automatic can
  switch the wait off.
- **A catalogue backup every day**, taken in the background when the app
  opens. The last week is kept, apart from the copies taken before an
  upgrade or a re-tag, which stay. Switch it off in Settings › Library.
- **Suggest picks, in Cull.** One button suggests a pick or reject for every
  frame you haven't judged yet:
  - the best frame of each burst is suggested as a pick, and the rest of the
    burst as rejects;
  - frames with missed focus, or that failed outright (black, or blown to
    white), are suggested as rejects;
  - every other frame is suggested as a pick.

  Suggestions show as dashed flags, each with its reason. Nothing changes
  until you accept: press **A** to accept one, or **Accept all** for the lot,
  which Ctrl+Z takes back in one go. **Suggested rejects** lets you check them
  before they're binned. A frame you've already rated or flagged is never
  given a suggestion. Copies of one photo in two folders are left for you to
  choose between.
- **Filter by camera, lens and exposure.** A new **Camera** section in the
  filters lists your bodies and lenses, with focal length, aperture and ISO
  in ranges ("51–105 mm", "ƒ/2.8 and wider", "ISO 1601–6400"). Each shows how
  many photos it would leave, like every other filter.
- **Filter by several people at once.** A **People** section lists everyone
  you've named. Tick two and you get the photos they're in *together*.
- **More ways to narrow by state**, in **Status**: edited, virtual copies,
  with faces, has a location, and no place named.
- **Sort the grid**: newest or oldest first, recently added, highest rated,
  file name, or sharpest. Clear all keeps your sort, and a saved view
  remembers it. Date headings show only when the order is by date.
- Every filter now has a chip above the grid, including folders and missing
  files, which had none, so nothing narrows the grid out of sight.
- **Faces up close, while you cull.**
  - **In Cull**, the panel beside the photo shows every face in the frame, with
    their name and whether their eyes are closed.
  - **In Compare and Duplicate review**, a Faces panel lines up each person
    across the set, so you can see who blinked in which frame. It also marks
    whose face is sharpest in each row and which faces are soft beside it.
  - Frames where everyone's eyes are open are marked **all open**.
- **Zoom the whole set at once.** In Compare and Duplicate review, **Z** or a
  double-click takes every frame to 1:1 at the same spot, and dragging one
  pans them all. Checking which frame of a burst is sharp now means looking
  at the same eye in each, side by side.
- **Sidecars carry your people, picks and captions, both ways.** An .xmp
  written here now holds, as well as the rating, label and keywords:
  - who is in the photo, by name, with a face region for each named face —
    the form Lightroom, digiKam and exiftool read;
  - pick or reject;
  - the caption;
  - the genre and conditions, filed under **Genre** and **Conditions** in
    Lightroom's keyword list.

  **Reading** a sidecar brings all of that in, from Lightroom and digiKam
  too. A named face region names the face found there, or puts the person in
  the photo if it hasn't been analysed yet, and the face is matched up when
  it is. A face you've already named is never renamed; the toast says how
  many were left alone. A caption someone wrote isn't replaced by the
  generated one.
- **Search understands people, places and dates.** Type "Anna in Edinburgh
  last summer, sunset" and press Enter: Anna, Edinburgh and last summer become
  filters, each with its own chip, and only "sunset" is searched by what the
  photos show. It knows:
  - the people you've named;
  - your places, and countries;
  - years, months, seasons, Christmas, "last week" and "yesterday";
  - your cameras and lenses;
  - a genre or condition that stands on its own: "portraits", "Anna, golden
    hour".

  A search that is all names and dates needs no AI model. When a word could
  be two things, such as a person and a place both called Paris, the other
  reading is one click away. **Search as typed** searches the whole text by
  content instead.
- **JPEG, PNG, TIFF and WebP photos are in the library.** Phone photos,
  JPEG-only cameras, scans and other apps' exports used to be skipped; now
  they scan in like raws, with search, faces, places (phones carry GPS) and
  culling, and a card of JPEGs imports. Develop opens them with every
  control, and says when a photo isn't raw, because there is far less to
  recover. A RAW+JPEG pair is still one photo.
- **iPhone HEIC photos.** HEIC and HEIF are read through the system's own
  codec, upright, with their date and place. On a Mac that needs nothing
  extra. Windows needs two free apps from the Microsoft Store for it (HEIF
  Image Extensions and HEVC Video Extensions); if they're missing, the scan
  says so instead of quietly skipping the photos.
- **Drop a folder on the Dock icon** to add it to the library, as you can
  drop one on the window. Folders still open in Finder as before.
- **Feels more at home on Windows.**
  - The title bar takes the app's own colours on Windows 11, so it reads as
    one surface with the top bar, keeping the system's buttons and Snap
    Layouts.
  - The keys a desktop app answers to now work there: Ctrl+O add a folder,
    Ctrl+Shift+I import, Ctrl+Shift+E export, Ctrl+, Settings, Ctrl+Alt+A
    Activity, Ctrl+W close Develop, the Lightbox or Compare, and F11 full
    screen. Rebind them in Settings.
- **Touchpads zoom and pan properly.** A two-finger scroll over a photo used
  to zoom it wildly. Now a mouse wheel zooms a step, a pinch zooms smoothly,
  and a scroll moves a zoomed photo around, in the Lightbox, the loupe and
  Develop.
- **AI noise reduction, in Develop.** Under Detail, **Reduce noise** clears
  colour speckle and blotchy high-ISO noise while keeping edges sharp,
  leaving a fine, even grain the **Luminance noise** slider can take down. It
  runs on your graphics card: about 2 seconds for a 24-megapixel photo on a
  fast one, a minute or two on a laptop's built-in graphics, and several
  minutes on a computer with no graphics card to use. The
  panel says how long it will take on yours, and how long is left while it
  runs; you can keep editing meanwhile. Zoomed in, it starts with the part
  you're looking at, and a long run shows the result filling in as it goes. It runs once per photo; after that
  **Amount** mixes it in as you move it (all of it to start). It is saved with the edit, can be undone, and exports and
  batch exports apply it. A photo reopened with it shows without it until you
  press **Run now**. A 234 MB download the first time (NAFNet, MIT).
- **Highlights.** Above the grid, **Highlights…** chooses the best of what
  you're looking at (a trip, a person, a shoot, a month), leaving out
  rejects, blinks and soft frames, picking for variety rather than ten of one
  sunset, and making sure everyone named in them appears. It saves the choice
  as a collection you can edit. Needs the search model.
- **Closed eyes are read properly now.** The old reading called most smiling
  faces "eyes closed". A new one (MediaPipe's face mesh) tells a smile from a
  blink, and says **Can't tell** for a profile, sunglasses or a face too small
  to read instead of guessing. Your library's faces are re-read in the
  background, from the faces already found, so names and groups stay as they
  are. With the reading trustworthy again, a burst's suggested keeper once
  more prefers a frame where the eyes are open, and Compare marks who blinked.
- **Unplaced faces, in People.** Faces too small, turned away or faint for
  grouping to place were found but shown nowhere. **Unplaced faces…** (in
  People's menu) lists them largest first: type who each is and press
  Enter, or mark it **Not a face**.
- **Choose a person's cover face.** In **Review faces**, hover a face and
  pick **Use as cover**; it stays through regrouping.
- **Page Up and Page Down in the grid** move a screenful of photos.
- **Masks do more: Whites, Blacks, Clarity and Texture.** A mask can now
  set its own white and black points and its own local contrast, the same
  way the global sliders do.
- **White balance picker in Develop.** Click the eyedropper beside Auto
  white balance (or press W), then click something in the photo that should
  be grey or white: Temperature and Tint are set so it is. A bright white
  wall works as well as a grey card. Esc puts it down.
- **Adobe RGB and Display P3 photos show in their true colours.** A JPEG,
  TIFF or HEIC's colour profile is read. An Adobe RGB export or an iPhone's
  Display P3 photo is converted for the grid and the loupe, and opens in
  Develop with its own colours, instead of looking washed out. sRGB photos
  are shown untouched, as before.
- **A scan says why photos couldn't be read**, once, instead of skipping
  them silently.
- **Shoots: each card import, from card to export.** An import is now
  remembered as a shoot, under its job name. **Shoots**, in the filters,
  shows each with how far it has got: how many photos, how many judged, the
  picks, and how many of those are edited and exported. Beside each is the one
  thing to do next — **Continue culling**, **Develop picks** or **Export
  picks** — and clicking a shoot shows just its photos.
- **Ctrl+K opens a command palette.** Type a few letters of anything: a
  command, a keyboard action (with its key beside it), a person, a place, a
  collection or a saved view. Enter runs it.
- **Keys that work anywhere:** Ctrl+1–4 for Browse, Cull, Places and People,
  and Ctrl+F to search. Like every key, they can be changed in Settings.
- **Copy and paste just some of an edit, in Develop.** **Ctrl+Shift+C** asks
  which parts — white balance, light, colour, curves, detail, lens, effects,
  look, crop, masks, spots — and **Ctrl+Shift+V** pastes them onto another
  photo. The crop, masks and spots start unticked, since they're drawn on one
  photo. **Copy to N selected** and **Save preset** ask the same, and a
  preset now carries only its parts, leaving the rest of a photo's edit
  alone. Presets saved before still apply as they did.
- **Paste previous**, in Develop: the last photo's settings onto this one,
  all but its crop, masks and spots.
- **History, in Develop**: every edit on the photo, to go back to any of them.
- **Rate, flag, label and step through photos inside Develop**, with the same
  keys as everywhere else (0–5, C, X, U, 6–9, ← →).
- **Name the people in a photo from the Lightbox.** A **People** section
  shows every face found in the photo, with who it is:
  - **Name…** on an unnamed face names its person. If someone already has
    that name, the two become one.
  - **Not them** on a named face takes it out of that person's photos.
  - Faces too small or turned away to group with anyone show as **Not
    grouped** here too — until now there was nowhere to see or name them.
- **Find and sort people** on the People page, by name or by number of
  photos. Type "unnamed" to find the people still to name.
- **The "Same person?" keys (Y, N, S) can be changed** in Settings, and show
  in Shortcuts.
- **Judge several frames at once in Cull's survey.** Ctrl-click (⌘ on a Mac)
  chooses a frame, Shift-click a run; then C, X, a rating or a colour label
  applies to every one of them, and Ctrl+Z takes it back in one go. Esc lets
  go of the choice.
- **A Rejects strip in Cull**, beside All, To review and Picks, to check what
  **Bin rejects…** would bin before it does.
- **Save your picks as a collection**, from the Picks strip in Cull.
- **Rename a collection** from the pencil beside it in the filters, and
  **take photos out of one**: with a collection showing, the selection panel
  offers to remove the selected photos from it.
- **Take a place off photos**, from the selection panel. The place itself
  stays, with its pin and its other photos.
- **Write or read the sidecars of just the photos you've selected**, from
  the right-click menu. Activity still does the whole library.
- **Exported JPEGs carry keywords, the caption and who is in them.**
  **No location** now leaves out the names as well as where it was taken.

### Changed
- **Exporting a batch is 1.7 times faster.** The next photo decodes while
  this one renders and the last one is written, instead of each waiting for
  the one before — 240 ms a full-size photo on an M4, down from 400.
- **Exporting with sharpening is a third quicker.** Sharpening now uses every
  core, with the same result: a 24 MP JPEG with Standard sharpening takes
  240 ms to encode instead of 350.
- **Searching by content no longer loads the whole library into memory.**
  Each photo is scored as it is read, so a search in a 50,000-photo
  library holds under a megabyte where it held over a hundred, and
  Highlights reads only the photos it is choosing from.
- **The window stays responsive while the library loads and filters.**
  Reading photos and their counts, search, people suggestions, places, XMP
  import and export and the folder list no longer run on the thread that
  draws the window, so a large library no longer holds up menus, the window
  or other clicks while it answers. A search typed while the library is being
  indexed no longer waits for indexing to finish.
- **Adding photos is up to twice as fast, and reads far less.** A raw is
  read only as far as its preview — 2 MB of a 48 MB Nikon NEF, 1 MB of a
  56 MB Fujifilm RAF — which matters most from a card or an external drive,
  and each preview is decoded once instead of twice.
- **Search and tagging use less than half the memory.** The search model
  loads each half when it is first needed, and tagging shares it rather than
  loading its own copy. A session that indexes and tags new photos now holds
  about 0.9 GB for it instead of 2.6; searching adds the rest.
- **Opening the app checks your folders without reading your photos.** The
  check for new photos at launch knows an unchanged file by its size and
  date, instead of opening every photo in the library to look — 13 times
  quicker on an internal drive, and far more on an external one. The first
  launch after updating still reads each photo once.
- **Develop, tidied up.**
  - **Looks starts closed**, so Develop opens on the photo rather than on a
    gallery of looks beside it. The **Looks** button, top left, opens it; it
    stays open while you move between photos.
  - **The bar along the top has three parts**: back to the Library and
    Looks on the left, the photo's name in the middle, and undo, redo,
    History, **Save as copy** and **Export** on the right. **Before / Split /
    After** and **Gamut warning** moved to the bar under the photo, beside
    zoom, and **Auto** moved into the adjustments.
  - **Sliders take one line each**: name, slider, value. The slider fills
    from its starting point to where it is, and a changed value shows
    brighter, so what you've moved stands out. Double-click the name or the
    slider to put it back — to where that control starts, which for
    sharpening radius or grain size isn't 0.
  - **Crop has its own panel.** Choosing Crop shows the aspect ratios
    (including **Original**), rotate, flip, straighten and **Reset crop**
    in the right panel, instead of in a closed Geometry group at the bottom.
    **Free** now just unlocks the ratio; it used to throw the crop away.
  - **Presence, Color mixer and Color grading are groups of their own**,
    each with its own reset, instead of three tabs inside Color that hid
    each other's changes. Noise reduction has its own heading under Detail.
    Groups remember which are open.
  - **Adjust, Crop, Mask and Retouch are one row of tools**, and a tool with
    edits in it shows it: a dot for a crop, a count of masks or spots.
  - **Copy, Paste and Previous sit at the foot of the adjustments**, always
    in the same place, with **Copy to selected** beside them when other photos
    are selected in the library. Previous used to appear in the top bar only
    after a second photo, pushing the other buttons along.
- **A large library opens and filters sooner.** Reading the grid's photos
  takes half the time it did: at 50,000 photos, the first 20,000 come back in
  87 ms instead of 184.
- **Grouping faces into people is over ten times faster.** It runs after
  every face pass, and at 10,000 faces took 11 seconds; it now takes under
  one.
- **Exporting from Browse or Cull opens the export dialog**, with every
  setting Develop's has. It used to export straight away with fixed settings
  (JPEG 92, full size, Standard sharpening), so the same photos came out
  sharpened or not depending on where you pressed Export.
- **The key hints in Compare and the Lightbox show your keys.** Rebind one in
  Settings and its hint follows. Compare's said "Pick (P)" while pick was C.
- **A collapsed burst shows its sharpest frame**, as it was meant to; it
  showed whichever frame came first.
- **The Develop filmstrip stays quick with a large filter.** It drew every
  photo at once, up to 20,000; now it draws only what's in view.
- **Content search shows the photos that match**, not always the nearest
  thousand. A search for something your library doesn't have now comes back
  (nearly) empty instead of filling the grid.
- **Typing in the search box also finds people's names and captions**, and
  is quicker than it was.
- **Double-clicking a frame in Compare zooms in**, as it does in Cull and the
  Lightbox. It used to open Develop; **D** still does.
- **Closed eyes no longer decide which frame of a burst to keep.** The eye
  check reads most smiles, small faces and turned heads as a blink, so the
  smiling frame, often the one worth keeping, was ranked last. Until it's
  fixed, the eyes badge still shows (and can still be dismissed), but focus
  alone picks the keeper, and Suggest picks never rejects a frame for its eyes.
- **The suggested keeper of a burst of people is the one with the sharpest
  face**, not the sharpest frame overall. A portrait's background is soft on
  purpose, and a busier background used to win.
- **Score focus is now Score frames**: it also measures how bright each frame
  is, which the suggestions use. Photos scored before this are measured again
  the next time it runs.
- **On-device AI sets itself up.** The first time you add photos, Northlight
  downloads what search, tags and People need (about 1 GB, once) and starts
  analysing. Progress and Cancel are in Activity.
  - One download instead of a question for each feature, and People no longer
    needs a download of its own the first time you open it.
  - On a metered connection, such as a phone hotspot, it asks first. So does
    the Descriptions model, which is several gigabytes.
  - The face detector now comes with the app, so checking for closed eyes in
    Cull works with nothing to download.
  - Select Subject and Click to Select download when you first use them,
    without a dialog, unless you're on a metered connection.
- **Better search, in your own language.** Search, Similar, Duplicates and the
  built-in tagger now run on SigLIP 2 instead of CLIP.
  - On a real library it finds the right photos noticeably more often.
  - It understands searches in French, German, Spanish, Italian and many
    other languages, which CLIP couldn't.
  - It's a one-off 785 MB download, after which your library is re-indexed
    in the background. Until a photo is re-indexed it can't be found by
    content search, but filters and descriptions still work.
- **Automatic analysis catches up at launch.** It used to run only after a
  scan found new photos, so a pass cut short by quitting — or a feature
  turned on since — waited for the next import.
- **Tagging with the vision model is three times quicker on a Mac.** With
  Detailed descriptions on, a photo's tags now take 2.3 seconds instead of
  8.2 on a 16 GB M4, so the filters fill as fast as you can look: the model
  reads a smaller copy of the photo for its tags, and works on two photos at
  once. Descriptions still read the full-size photo, so text in the frame
  stays legible, and take 6.3 seconds instead of 7.9. Tags come first;
  **Describe** writes the descriptions afterwards, and tags any photo it
  reaches without them in the same request.
- **On Windows, tags and a description come from one request**, so the
  vision model reads each photo once rather than twice when both are needed.
- **Settings › AI shows a download in the row that needs it.** Turning on a
  feature that fetches a model now shows the model arriving in that row, in
  megabytes, rather than only in Activity.
- **The Dock icon bounces when long work finishes in the background.** An
  export, scan or analysis that ran for more than 20 seconds asks for your
  attention once when it ends, if you've switched to another app (on Windows
  the taskbar button flashes).

### Fixed
- **The Mac app no longer fills your disk with hidden copies of its AI
  models.** Each time it quit after finding faces, it left behind the copy
  of the models it had prepared for the graphics chip, a few hundred MB at a
  time, in a temporary folder that nothing cleared: one Mac had 4.4 GB of
  them. The app now prepares each model once, keeps it in its cache and
  reuses it, so face finding also starts sooner. It removes the copies that
  earlier versions left behind when it starts.
- **Select Subject and click-to-select no longer freeze the app.** The
  window, its menus and Develop's sliders stayed stuck for the second or two
  the model took (on the first click, for click-to-select). Develop now stays
  live while it works; open another photo or change the crop meanwhile and it
  asks you to select again rather than placing the mask wrong.
- **Rating a photo no longer replays the grid's entrance.** Every rating,
  flag or colour label made every photo in view fade in again, as if a new
  filter had been applied.
- **AI features use your faster graphics card.** On a laptop with both
  built-in and dedicated graphics, face finding, search, subject selection
  and noise reduction all ran on the slower built-in chip. They now use the
  same graphics card as Develop.
- **Automatic analysis no longer stops after a scan.** Now and then a scan
  that had finished still looked unfinished, and tagging and face analysis
  waited for it for good. It showed as new photos that were searchable but
  never tagged.
- **The white balance eyedropper's balance is kept.** Picked last, it was
  lost when the photo was closed.
- **Detailed descriptions download the model they need.** With another size
  of the same model already downloaded, the app took it for the one it
  wanted, and every description failed. The Describe button now also says
  why a run stopped, where it used to show nothing.
- **Quitting frees the vision model's memory.** The model could stay loaded,
  several gigabytes of it, after the app had quit. On Windows it is now freed
  even if the app crashes.
- **Descriptions no longer fill memory as they run.** The vision model's
  server kept every photo's prompt in memory, up to 8 GB of it, which pushed
  a 16 GB Mac into swap.
- **A Mac is no longer told its graphics suit only a compact AI model.**
  Apple silicon is judged by its memory, which the models share.
- **Auto-advance no longer skips a frame in To review.** Judging a frame there
  takes it out of the list, which already brings the next one up; moving on
  as well jumped past it.
- **A saved view with a person in it still shows that person after People is
  regrouped.** Regrouping renumbered everyone, so the view could show
  someone else, or nobody.
- **The WebP export no longer offers a quality it ignores.** WebP is saved
  lossless; the slider changed nothing.
- **Settings › Library says what's true about your files:** originals are
  never edited, and files are only written beside them (sidecars) or moved to
  the bin when you ask.
- **A virtual copy no longer overwrites its original's sidecar.** Both share
  one file, so writing sidecars left whichever came last. The original's is
  written; selecting a copy writes its original's.
- **Writing sidecars no longer deletes what Lightroom put in them.** **Export
  all** used to make every existing .xmp match the catalogue. So a photo you
  hadn't rated here lost its Lightroom rating and colour label, and its
  keywords were replaced with this app's. Now a sidecar keeps whatever was set
  elsewhere: a rating or label set here still replaces the old one, and
  keywords are added to the ones already there. Clearing a rating here still
  clears it from a sidecar this app wrote. A sidecar that can't be read is
  left alone and reported, not overwritten.
- **A damaged catalogue repairs itself from its backup.** If the catalogue
  can't be read at startup, Northlight keeps the damaged file, restores the
  newest backup that passes a check, and tells you what it restored. Before,
  the app either failed to start or opened with an error and no photos. A
  check after startup also looks for damage deeper in and offers the backup.
- **A failed backup can no longer push out a good one.** A backup that failed
  used to leave an empty file that counted towards the three automatic backups
  kept, so a few failures could delete the last good copy.
- **Only one copy of Northlight runs at a time.** Opening it again brings the
  running window forward instead of opening the same catalogue twice.
- **Restoring a backup no longer damages the catalogue it replaces, or loses
  your latest work.** If Northlight hadn't closed cleanly before the restore,
  the copy it kept of the replaced catalogue could be damaged, and changes
  made just before could be lost. Now it keeps a complete copy, including
  those changes.
- **Exported photos no longer look washed out.** The colour profile embedded
  in sRGB and Display P3 exports described the tone curve the wrong way round,
  so browsers, Photoshop, Lightroom and other colour-managed apps showed every
  export milky and flat. **Re-export anything exported in sRGB or Display P3
  before this fix**; Adobe RGB exports were not affected. The same fault gave
  the full-resolution zoom its white haze.
- **Zooming in waits less.** Full resolution is fetched once a photo has been on
  screen for a moment, not when you zoom. On Sony and Panasonic raws this is a
  1–2 second render.
- **No blank window at launch.** The window now appears once it has something
  to show, instead of sitting empty while it starts up.
- **The month bar over the grid no longer leaves a gap.** The pinned month
  sat 8 px below the top, so photos scrolled past above it, and the grid
  jumped when it appeared.
- **Analysis shows its progress where it says it does.** The Activity panel
  said "progress below" while nothing was below it. The current step's
  progress now shows in place, with "loading the model" until the first
  photo is done. Search indexing also starts sooner, because the tag
  vocabulary is only prepared when tagging needs it.
- **People are grouped properly.** The same person used to be split across
  several groups, and some groups mixed strangers. There are three causes:
  - **The face model.** It barely told people apart in real photos, so it
    has been replaced with a much stronger one. That is a one-off 261 MB
    download, asked for the first time you open People; culling still needs
    only the 1 MB face tools. Your photos are then re-checked in the
    background.
  - **The grouping method.** It snowballed: once a group mixed two people,
    it pulled in more. It no longer does.
  - **Poor views of a face.** Profiles and half-hidden faces now join a
    person but can't link two strangers.

  Faces on the People page are sharp portraits cut from the full preview,
  not blurry crops of the thumbnail. Each person's cover is their biggest,
  most face-on photo rather than whichever was "sharpest" (which could be
  the back of a head). Names, merges and faces you moved out are kept. When the app thinks two
  groups may be one person (say, with and without sunglasses), People asks
  **Same person?** and remembers the answer.

### Changed
- **Better descriptions from less memory.**
  - The model that writes photo descriptions is now Qwen3-VL for every GPU size: 4B for most GPUs, 8B for 12 GB and up, and 2B for small GPUs.
  - On a real library, the 4B describes as well as the previous 7B in 4.6 GB of VRAM rather than 6.7, and tags faster.
  - The 8B names landmarks and reads signs.
  - If you already use Qwen2.5-VL 7B, nothing changes until you pick another model.

### Added
- **Place photos from a GPX track.** For a camera with no GPS: record a
  track on your phone or watch, then choose **Place from a GPX track** in
  Places. It works out your camera clock's offset from UTC for you (you can
  nudge it), shows how many photos it would place on each day, and writes
  nothing until you say so. Gaps in the track are not guessed across.
- **Places from GPS, and a map.** Photos that carry a position — from a
  phone, a drone or a camera paired with its app — are placed in the town
  they were taken in, named offline ("Edinburgh, United Kingdom"), and drawn
  on a new **Map** in Places; click a place to see its photos. A place you
  set by hand is never replaced, and one you take off stays off. Photos
  already in the library are checked once, in the background, or from
  **Read locations** in Places. Nothing is looked up online. Place names
  from GeoNames (CC BY 4.0).
- **Import from a memory card**, from the folder menu. Photos are copied
  into dated folders of a library folder (optionally with a job name, and
  renamed by date), each copy checked against the card, with an optional
  second copy to another drive. Anything already imported is skipped, so a
  card imported twice copies nothing new. The card is only ever read. New
  photos are then scanned and analysed like any other.
- **New photos are analysed as they arrive.** After a scan adds photos they
  are indexed for search, tagged and checked for faces in the background, so
  describing a shot finds this week's photos with no further clicks. It uses
  only what is already set up (the first time, it offers the search download
  once), waits while you are in Develop, and can be switched off or run by
  hand from the Activity panel.
- **A library of several folders.** Add as many folders as you keep photos
  in — an internal drive and an archive disk, say — from the folder menu in
  the top bar. Each shows its photo count and anything missing, and can be
  rescanned, **located** after it moves to another drive, or removed. Filter
  by folder, or by **File missing**, in the filter rail. Adding a folder
  never replaces the library any more.
- **Right-click a photo** for Open in Develop, Find similar, **Show in
  Explorer**, Copy file path, rating, pick/reject and Export. The menu key or
  Shift+F10 opens it from the keyboard.
- **Move rejects to the Recycle Bin**, from Cull or the right-click menu, so a
  finished cull frees the disk. Sidecars and RAW+JPEG JPEGs go with them, the
  catalogue is backed up first, and everything can be restored from the
  Recycle Bin. A reject with a copy you haven't rejected is kept.
- **Vibrance and Saturation** sliders in Develop → Colour → Presence. Vibrance
  lifts muted colour first and leaves skin and already-vivid areas alone.
- **Portrait crop ratios** — 3:4, 2:3 and 9:16 alongside the landscape ones.
- **1:1 zoom in Cull and the Lightbox**, so focus can be judged before a verdict.
- **Catalogue backups** you can restore from, taken automatically before every
  upgrade that changes the catalogue's format.
- A **crash log** written when something goes badly wrong, so it can be
  diagnosed.
- **Settings → About**, with the version and a check for updates. Updates
  install themselves once they are being published; until then it says so.
- **Export in Display P3 or Adobe RGB** as well as sRGB, each with its colour
  profile embedded, and a **Perceptual** rendering intent that keeps vivid
  gradients from flattening where they run past the chosen colour space.
- **Exports keep their metadata** — camera, lens, exposure, capture date, and
  the creator and copyright set in Settings → Export — with a per-export
  **Strip** option.
- **Cancel** on every long job in the Activity panel: model downloads, the
  local AI runtime, search indexing, face analysis and people grouping.
  Stopping people grouping leaves your people exactly as they were.
- The **licence agreement** is shown by the installer and on first run, and
  can be read again from Settings → About → Licences.

### Changed
- **One search box.** Type to filter by filename, tag and place as before;
  press **Enter** to search by what the photos show, with the same words. The
  Text / Content / Similar switch is gone — **Find similar** is on the photo
  now, in the selection panel and the Lightbox.
- **Cull's key hints** fade once the pointer rests, instead of sitting over
  the bottom of the photo you are judging; move the mouse to see them again.
  Survey no longer repeats itself in a filmstrip, and Cull has its own icon
  rather than Search's magnifier.
- **Settings, simplified.** AI settings are down to three decisions: whether
  to use on-device AI, whether to turn on **Detailed descriptions** (the
  vision model, a several-gigabyte download), and what to delete to get space
  back. The provider, model, server-address, runtime and parallelism controls
  are gone — the app works them out. Catalogue backups moved to a new
  **Library** tab, and diagnostics to About.
- **Tagging uses the built-in model by default**, so the first AI download is
  one ~600 MB model that serves search, tags, Similar and duplicates. The
  vision model is only fetched when you turn on Detailed descriptions.
- The app's own AI runtime is **a fixed, tested version** that runs on **a
  port of its own**. It no longer uses, downloads into, or blocks an Ollama you
  have installed yourself, and the download size it quotes is the real one.
- The **filter rail** answers in tens of milliseconds at 50,000 photos (it took
  seconds), and filtering in general is faster on large libraries.
- **Near-duplicate grouping** — Review duplicates, Stack bursts, Select
  duplicates — takes well under a second on a whole library instead of minutes.
- The **grid** starts showing a large library far sooner.
- A **natural-language search** does its work once per search rather than twice.
- **Thumbnails** fill the grid about three times faster after a big jump
  through the library, and People's covers appear sooner.
- Local AI **sets itself up** instead of asking to be administered.
- **A new look.** The interface is quieter and more like an instrument than
  an app: neutral greys with no colour cast, **IBM Plex Sans** for text and
  **IBM Plex Mono** for numbers, white for what is selected, one clear primary
  button per screen, sentence-case labels, and far fewer outlines. AI features
  are named for what they do rather than marked with a sparkle.

### Fixed
- **Develop no longer loses edits** when you press Esc, go back to the
  Library or click another frame without saving: edits now save as you make
  them, and the Save button is gone. **Save as copy** keeps the look on screen
  as a copy, puts the original back to how it was when you opened it, and
  carries on editing the copy.
- AI can no longer be pointed at **another machine's** server: photos never
  leave this one, as the app has always said.
- Every **AI model the app can download is licensed for commercial use**.
  Qwen3-VL 2B and 4B replace Qwen2.5-VL 3B (research-only licence), MiniCPM-V
  (needed registration with its publisher) and Moondream; each model's licence
  is shown beside it. Downloads are pinned to a fixed version and always
  checksum-verified.
- **Renaming or moving a folder no longer loses the work on its photos.** The
  next scan recognises each file at its new path and brings its ratings, tags,
  places, faces and edits along, instead of adding it again as a new photo.
- A photo whose file has gone — its drive unplugged, say — is marked **File
  missing** and keeps all its work, rather than failing when it is opened. It
  comes back when the file does.
- The **crop box** no longer jumps around the frame while a corner is dragged.
- Canon, Fujifilm, Olympus and Panasonic photos show their **camera metadata**.
- **Content search** on a fresh install says why it has nothing yet, instead of
  returning silence.
- An unexpected error in one part of the app no longer **blanks the whole
  window** — that part says what went wrong and the rest keeps working.
- **Failed sidecar writes** are reported instead of being silently miscounted.
- One AI failure can no longer **disable every AI feature** until a restart.
- **People, face crops and places** were placed at the wrong scale.
- The app now says correctly **where your catalogue lives** — in the app's
  data folder, not beside your photos.
- A **look (LUT)** on a Display P3 or Adobe RGB export now changes the colours
  it was made to change; it was being shown the wrong colour encoding.
- At the smallest window size the **Export dialog**'s Export button was below
  the bottom edge, out of reach. The dialog now scrolls.
- **Trip names in Places** no longer shrink to a couple of letters in a small
  window.
- The **keyboard hints** over the Cull loupe no longer spill out of their
  labels in a small window.
