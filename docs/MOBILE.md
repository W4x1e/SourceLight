# SourceLight on phones

Status: agreed 2026-10-02, being built. Not for the first release.

Built so far (2026-10-02), each part stacked on the one before:

- The link crates (`sl-protocol`, `sl-link`): pairing and connections.
- Desktop Settings → Phone, behind the `phone-link` feature.
- The phone app's first build (Android): pairing, link status, a test
  grid and loupe.
- Library sync: the desktop's change feed (migration 49), the phone's
  copy of the library, thumbnails and previews fetched and cached, picks and
  rejects in both directions.
- Shoot track (Android): a plugin (`mobile/plugins/shoot-track`) records a
  fix every 10 seconds from a foreground service, with the screen off. A
  stopped track is sent as GPX; the desktop keeps it in `tracks/` and offers
  to place the shoot's photos through the Places track dialog.
- QR scanning on ZXing (`mobile/plugins/qr-scan`), replacing the official
  plugin's closed-source ML Kit.
- Send to phone: the Export dialog's "Send to phone" button (shown once a
  phone is paired) runs the selection through the usual batch export, with
  the dialog's settings, into a folder under `outbox/` beside the catalogue
  (migration 50 lists the files). The phone fetches the list on every sync,
  saves each photo to the gallery (`mobile/plugins/gallery`: MediaStore into
  Pictures/SourceLight; Android 9 and earlier ask for storage access) and
  tells the desktop after each one, which then deletes it. Only folders
  inside `outbox/` can be queued, so the page cannot offer any other file to
  a phone. Retry and Show folder are hidden in the export queue for these.

- GPU test (section 4, step 1): the desktop's Develop pipeline runs in the
  phone app. On a Pixel 8 Pro (Mali-G715, Vulkan) it meets every limit
  and renders a screen-sized frame in 20 to 30 ms (results under step 1).

- On this phone (section 4, step 3): the phone's own photos in their own
  tab, by album, with limited access handled. Android only so far.
- Develop on the phone (section 4, step 4): Light, Colour, Crop and Looks on
  the phone's own photos, edits kept as settings, Save copy to the gallery.
- The shared develop crate (section 4, step 2), and the phone app on
  it.
- Add to library (section 4, step 5): phone photos, with their edits, into
  the desktop's library on the next sync.
- Editing library photos (section 4, step 6): non-raw library photos edited
  on the phone, the edits going to the computer, and onto a virtual copy if
  the photo was edited there meanwhile.
- Ratings: stars in the library loupe, and on grid tiles, sent to the
  computer like picks and rejects.
- Film looks: the desktop's built-in film LUTs in the phone editor's LUTs
  tool, with their strength. They are made in code (`lut_packs::texels`, the same
  table the desktop's seeded file loads as), so the phone needs no files, and
  a library photo using one stays editable on the phone. Imported LUTs stay
  on the computer.
- Edited previews: the library loupe shows an edited photo as edited, raws
  as the computer develops them. The desktop renders it on request at
  preview size (`ImageSize::Edited`, one at a time, without AI noise
  reduction), keeps it in `thumbs/edited/` by its edits' tag, and the phone
  caches it the same way; the plain preview stands in when it can't be had.
  Grid tiles stay as the desktop's thumbnails, unedited, as on the desktop.
- Effects in the phone editor: texture, clarity and dehaze, the vignette,
  grain and its size, and halation, with the desktop's labels and ranges.
- Crop by dragging: the Crop tool shows the photo uncropped (still turned,
  flipped and straightened) with the desktop's crop box over it, its grips
  sized for a finger; a ratio chosen below holds while dragging, or Free.
- Search: Search in the library header. The phone reads the query as the
  desktop's search box does (`readQuery` with the computer's vocabulary:
  people, places, dates, gear), and the computer runs it with the same photo
  query its grid uses, picture search and captions included
  (`Request::Vocabulary`, `Request::Search`). Results show as the grid,
  best first for a search by content. Needs the computer connected.
- The screens in the desktop's design (DESIGN.md), at a finger's size:
  - Three tabs along the bottom: Library, On this phone and Tracks (GPS
    tracks). Each keeps its place; tapping the tab showing scrolls it to the
    top. Navigation follows the platform's photo apps, as researched on
    2026-10-05: tabs for places, a sheet for the link, Cancel and Done in
    the editor.
  - Each tab has a top bar with the computer button at its right (and
    Search in the library), and a large title above the grid that scrolls
    away with it; the bar then shows the title. The library's title is what
    it shows (all photos, picks, unflagged, rejects, rated, edited, each
    with its count) and opens a menu of the others. On this phone, the
    title is the album. Beside the title, the order: newest or oldest first.
    Under it, the library's status (a green, amber or red light, the count
    and when it last synced).
  - The computer button carries the link's light, and spins while a sync
    runs. It opens the sync sheet (`mobile/src/link.tsx`): the computer, its
    connection, when it last synced, what is waiting to send (decisions,
    edits, photos to add, tracks: `link_waiting`), Sync now and Forget
    computer, which asks first. Unpaired, it offers Pair instead. Holding
    the version line shows the tests. One `LinkProvider` runs the link for
    every tab: it syncs every 10 s and on Sync now, and tells the tabs when
    a sync ends.
  - Pulling the library's grid down from its top syncs, as Mail and Photos
    do. Offline, the library says so in one line with Retry.
  - Sheets (`Sheet`, `ConfirmSheet` in `mobile/src/ui.tsx`) rise from the
    bottom over a scrim. Drag the grabber down, tap outside or press Back
    to close one.
  - On this phone, Select or a long press on a photo starts choosing photos
    to add to the library.
  - The grids have month headings, and the tiles have the desktop's
    white-on-black chips.
  - The loupe's top bar has Back (the arrow at its left) and shows the
    date, position and file name. Under the photo: Reject, Pick and Edit,
    with Edit as the one key button. A raw shows why it can't be edited
    here instead. A phone photo has Add to library and Edit.
  - The sliders are the desktop's, and the type is a step up from the
    desktop's sizes.
  - Shared pieces live in `mobile/src/ui.tsx`.
- Gestures:
  - Pinch a grid to fit more or fewer across, from one fewer than fit to
    three more. The grid scales with the fingers and lays out when they
    lift, keeping the photo under them.
  - In the loupe, pinch or double-tap zooms about that spot, and panning
    stops at the photo's edges. Pulling the photo down closes it.
- Landscape: on a phone on its side, the loupe and the editor put their
  controls in a column beside the photo. Grid and loupe side by side is for
  tablets only (a screen whose shorter side is 600 px or more, and 900 px
  wide). There the loupe pane has Full screen and Close at its top right,
  and says "Choose a photo to see it here" when none is chosen. Back, Escape
  or Close lets go of the photo; from full screen, Back returns to the
  pane. The arrow keys move between photos.
- Third-party notices: `mobile/THIRD-PARTY-LICENSES.txt`, generated by
  cargo-about for the Android arm64 build (`mobile/src-tauri/about.toml`,
  `mobile/about.hbs`) with the JavaScript, font and Android parts written in
  the template. The app shows it as Open-source licences in the sync sheet, and
  `check.sh --mobile` fails when it is stale.
- Without a computer: the pairing screen's "Use without a computer" opens
  on the phone's own photos. The library tab says it isn't paired and offers
  Pair. The scrolling spike's stand-in grid is Scroll test among the tests
  in the sync sheet, while not connected.
- The phone editor, with the desktop's tools and labels:
  - Cancel (✕) at the top left and Done at the top right, as in Photos.
    Changes stay in the editor's undo history until Done, which keeps them
    (`edit_done`): a phone photo's in `device.db`, a library photo's sent to
    the computer. Cancel with changes asks before discarding them, as does
    Back. Undo, redo and compare sit between the two.
  - Tools in three groups along the bottom, a segmented control: Adjust
    (Light with the histogram and Auto tone, Colour with Auto white balance
    and the picker, Curve, Mixer, Grading, Detail, Effects, Lens), Presets
    (Presets and LUTs, as the desktop's Presets panel has them) and Crop.
    The group's tools sit in a strip above it, and each group comes back to
    the tool last used in it.
  - Presets lists the built-in presets by group, each opening onto its
    presets as cards rendered on the photo, and LUTs shows the film LUTs
    the same way (`editframe` frames, which `img-src` now allows).
  - Auto, Auto WB, the picker and Paste run in the shared crate as on the
    desktop (`edit_auto`, `edit_pick_wb`, `edit_paste` with
    `take_groups`, which leaves the crop behind).
  - Each tool has a dot when it has changed the photo, and its own Reset
    (`restOf`, `isMoved` in `mobile/src/EditPanels.tsx`).
  - The ⋯ menu has Save a copy (a phone photo), Copy edits, Paste edits and
    Reset all. Reset all keeps masks and retouching made on the computer.
  - Compare, or holding the photo, shows the original in the edit's crop.
  - A slider snaps to its rest near it, and a double tap anywhere on its
    row resets it.
  - Pinch or double-tap zooms in, to twice the photo's own pixels at most,
    and dragging pans. While the fingers move, the frame there is scaled to
    follow. Then a frame of just the part showing is rendered
    (`editframe?k=&cx=&cy=`, the desktop's `ViewPort` and `render_view`).
  - Each opening has a token that every change carries, so an editor that
    is closing can't touch the next photo.
  - If the photo went from the GPU, it opens again with the changes made
    so far. Changes not yet kept with Done live only in the running app, so
    they are lost if the system ends it in the background.
  - Sliders and crop grips keep clear of the sides where a swipe is the
    system's Back. MainActivity passes the gesture insets as
    `--gesture-left` and `--gesture-right`.

Next: real-phone measurements, on Android and an iPhone; scanning the pairing
code on iOS. The app runs on the iOS simulator since 2026-10-05 (step 7).

This settles three things for the iPhone and Android apps: what they do, what
they are built with, and how a phone links to a desktop. The short version:

1. **What it does.** The phone is a companion to the desktop, and an editor
   for photos that are not raw. The desktop keeps the catalogue and does the
   heavy work (raw decoding, AI, export). The phone carries previews, collects
   decisions (picks, rejects, ratings, tags) and field data (GPS tracks, phone
   photos), and receives finished exports. Version 1 is: browse, cull, log a
   GPS track during a shoot, and send exports to the phone, at home or away,
   on iPhone, iPad and Android. Next comes editing on the phone (section 4):
   the phone's own photos as a second library, edited on the phone with the
   desktop's own pipeline, and added to the synced library when chosen.
2. **Stack.** A second Tauri 2 app, built for iOS and Android, in this repo.
   Rust core plus React and TypeScript, sharing Rust crates, translations and
   pure logic with the desktop. Small Swift and Kotlin plugins for background
   GPS and the photo library. Expo (React Native) with the same Rust crates is
   the fallback if the spikes fail.
3. **Linking.** No accounts. A phone pairs by scanning a QR code on the
   desktop. Each device has an Ed25519 key, and the desktop accepts only keys
   it has paired. Transport is iroh (QUIC, end-to-end encrypted). Away from
   home, the phone reaches the desktop by its key through a relay we run. The
   relay passes encrypted traffic and stores nothing, so it needs no accounts
   either.

---

## 1. What the phone app does

### The rule

The desktop is the source of truth for the library. The phone never runs
LibRaw or ONNX models. It holds a replica of the catalogue's small fields and
a cache of images the desktop already made. Every change it makes to the
library goes back to the desktop as an operation, applied the same way a click
on the desktop would be. The one heavy part the phone runs is the Develop
pipeline, for photos that are not raw (section 4).

This keeps the phone app small, keeps one place where the truth lives, and
fits the product principles: the originals are never written, human decisions
outrank the machine, and no account is needed.

### Version 1

| Feature | What it does | Why it earns a place | Reuses on the desktop |
|---|---|---|---|
| **Pair** | Scan a QR code in Settings → Phone. | Everything else needs it. | New: link service, `paired_devices` table. |
| **Browse** | Shoots, collections, dates, and filters on rating, flag, people, places, tags. Thumbnails offline; previews fetched and cached. | Looking at the library away from the desk is the most basic ask. | Existing 512 px thumbnails and 1600 px previews, no new rendering. |
| **Cull** | Full-screen loupe. Swipe or tap to pick, reject, rate and label. Burst groups with the suggested keeper and focus band. Works offline; decisions sync on the next connection. | Doing the first pass on the sofa or a train is the strongest reason to install the app. | Sharpness, burst groups and suggested keepers are already computed. |
| **Shoot track** | Record a GPS track while shooting, screen off. Sent to the desktop and applied to that shoot's photos. Can also be exported as a GPX file. | Most dedicated cameras record no position. `gpx.rs` already places photos from a track and guesses the camera's clock offset; today the user has to find a logger app and move the file by hand. Also makes the app useful on its own, which helps App Review. | `gpx.rs` (`parse`, `best_offset`), the Places workspace. |
| **Send to phone** | Pick photos on the desktop, choose an export preset, and they land in the phone's photo library. | Edit on the desktop, post from the phone. Today that is email, a cable or a cloud folder. | The export pipeline. |
| **Away from home** | Everything above works from anywhere the desktop is reachable, through our relay when a direct connection is not possible. | Without it the phone only syncs at home. Decided 2026-10-02: remote access is required. | Nothing; the link handles it. |
| **iPad layout** | The same app with a two-pane layout: grid beside loupe. | Culling on a tablet is good. Decided: in version 1. | Nothing. |

### Version 1.x

- **Search from the phone.** When connected, the phone sends the text and the
  desktop runs the SigLIP 2 search and returns ranked ids. Offline, the phone
  searches tags, captions, places and people in its replica (SQLite FTS5).
  Shipping the text encoder to the phone is not worth its size.
- **Phone photos into the library.** Now part of editing on the phone
  (section 4): Add to library from On this phone. Videos are skipped, as the
  desktop has no video support. Check that LibRaw decodes current ProRAW DNGs
  before promising them.
- **People queue.** Swipe through unnamed faces and name them. Needs face crops
  served by the desktop.
- **Camera clock check.** The phone shows a large QR code holding the exact
  time. Photograph it with the camera; the desktop reads it from that photo and
  knows the camera's clock offset to the second, so `best_offset` no longer has
  to guess.
### Not doing

| Idea | Why not |
|---|---|
| Raw develop on the phone | LibRaw is LGPL, which is awkward on iOS, and raws are 25 to 60 MB each to move. Photos that are not raw are edited on the phone instead (section 4). |
| AI analysis on the phone | Models are hundreds of MB, battery cost is high, and the desktop already does it. |
| Cloud sync, cloud backup, hosted galleries for clients | Needs servers that store photos, accounts, and a recurring cost that means a subscription. Breaks "no account" and "no subscription". |
| Push notifications ("analysis finished") | Needs Apple and Google push servers and a backend of our own. Small value. |
| Phone as the main photo app | The phone's own photo app does this. On this phone (section 4) is for picking photos to edit, not a replacement gallery. |

---

## 2. Tech stack

### Decision

A **second Tauri 2 app** in this repo, built for iOS and Android.

- **Rust core** for the replica database (rusqlite, bundled), sync, the link,
  pairing and the image cache.
- **React and TypeScript** for the interface, sharing translations (all five
  languages), design tokens and the pure logic in `src/lib/` (for example
  `survey.ts`, `queryReading.ts`) with the desktop.
- **Shared Rust crates** used by both apps: `sl-protocol` (message types and
  versions) and `sl-link` (the iroh endpoint, pairing, authentication). The
  desktop crate depends on them like any other crate.
- **Native plugins** written in Swift and Kotlin, each small:
  - background GPS logging (iOS `CLLocationManager` with background updates;
    Android a foreground service of type `location`);
  - saving to the photo library and the system photo picker;
  - Bonjour browsing for re-finding the desktop (iOS `NWBrowser`, Android
    `NsdManager`).
  - QR scanning: our own small plugin (`mobile/plugins/qr-scan`) on ZXing,
    Apache-2.0. On iOS, AVFoundation reads QR codes itself.

### Why

| | Tauri 2 mobile | Expo (React Native) + UniFFI | Swift + Kotlin | Flutter or Compose Multiplatform |
|---|---|---|---|---|
| Rust shared with desktop | Directly, no bindings | Through generated bindings | Through generated bindings | Through generated bindings |
| React, TS, translations shared | Yes, components too | Logic and translations; components rewritten | No | No |
| Feels native | Good for grids and a loupe; navigation and gestures need care | Good | Best | Good |
| Background GPS, photo library | Write two small plugins | Mature Expo modules | Native | Plugins exist |
| Maturity on mobile | Newer than desktop, rough edges in plugins, signing and webview quirks | Mature | Mature | Mature |
| New things to learn | Snippets of Swift and Kotlin | React Native, Expo, its build service | Two UI toolkits | A new language |

The deciding points for one developer at 10 to 15 hours a week:

- The sync, crypto, protocol and SQLite code is Rust under every option. Tauri
  is the only one where it needs no binding layer.
- The app is grids, a loupe and lists. A webview handles those well; the
  desktop is already one.
- The risky native parts (background GPS, the photo library) are small and
  would be written natively under any option.
- One UI language, one build system, one set of translations.

The honest risk is mobile maturity. Tauri's mobile targets are newer than its
desktop ones. That is why the work starts with spikes.

### Spikes first (about a week each)

1. **Grid.** A 20,000-tile virtualised grid with thumbnails from a custom
   protocol, on an iPhone and on a mid-range Android. Pass: smooth fling
   scrolling, and pinch-zoom in the loupe that feels native.
2. **Link.** iroh inside the Tauri mobile Rust core on both phones. Dial the
   desktop by address, move 1,000 thumbnails. Pass: it builds for both, and the
   transfer takes seconds, not minutes.
3. **Shoot track.** The background GPS plugin, one hour with the screen off on
   both phones. Pass: a point at least every 10 seconds, and battery use a
   photographer would accept.

If spike 1 fails, keep the Rust crates and build the interface in Expo with
UniFFI bindings (`uniffi-bindgen-react-native`, MPL-2.0, which `deny.toml`
already allows). Spikes 2 and 3 carry over unchanged. Only the UI layer is
lost.

### Repo layout

```
src-tauri/         the desktop, and the Cargo workspace root
  crates/
    sl-protocol/   message types, protocol version
    sl-link/       iroh endpoint, pairing, peer allow-list
    sl-develop/    planned (section 4): the edit model, GPU pipeline and image
                   decoding, shared by both apps
src/               the desktop interface
mobile/
  src/             the phone interface (imports shared src/lib, i18n, tokens)
  src-tauri/       the phone's Rust core and plugins
```

The shared crates sit inside `src-tauri/` so the desktop's `Cargo.lock`,
`target/` and release profiles stay where they are. `scripts/check.sh` lints
and tests them; `cargo deny` already walks the whole workspace.

iOS builds need Xcode, so they run on the Mac mini. Android builds run on
Windows.

---

## 3. Linking

### Accounts are not needed

An account usually buys four things. Each has a way without one:

| What an account would do | How it works without one |
|---|---|
| Prove the phone is yours | Pairing. The phone scans a QR code that only your desktop's screen shows. From then on, each side knows the other's public key. |
| Find the desktop | On the same network, the QR code carries its addresses, and Bonjour re-finds it if they change. Away from home, iroh dials by public key. |
| Keep traffic private | QUIC with TLS, authenticated by the same keys. Nothing is readable in between, including by a relay. |
| Recover after losing a phone | The desktop holds the truth. Remove the lost phone on the desktop and pair the new one. |

Not having accounts also avoids a list of obligations: a server holding
personal data, password resets, GDPR requests, a breach to worry about, and
Apple's rule that apps with account creation must offer account deletion. The
App Store privacy label can say "Data Not Collected".

**When accounts would become necessary:** hosted features. Cloud backup of the
catalogue, galleries for clients, syncing while the desktop is off, or teams.
Each needs a server that costs money every month to run. If any of them is
ever built, it should be a separate opt-in service, not a requirement of the
phone app.

### Pairing

1. On the desktop, Settings → Phone → **Pair a phone**. The desktop starts its
   link and shows a QR code: its public key, its local addresses, a one-time
   token (128-bit, valid five minutes), and the computer's name.
2. The phone scans it, makes its own key pair (kept in the Keychain or the
   Android Keystore), dials the desktop and presents the token and its key.
3. The desktop asks: "Allow 'My iPhone' to connect?" One click.
4. The desktop stores the phone's key in a new `paired_devices` table (name,
   key, paired, last seen). The phone stores the desktop's key, the catalogue
   id and the last addresses.
5. From then on the desktop accepts only paired keys. Settings → Phone lists
   paired phones with **Remove**. A removed phone is told so on its next
   connection.

The link runs only while SourceLight is open and phone connections are on.
They are off until the first pairing.

### Transport: iroh

iroh 1.0 shipped in June 2026 with a stable wire protocol; 1.3 is current. It
is a Rust library (MIT or Apache-2.0) that connects devices
by Ed25519 public key over QUIC. It connects directly when it can and through a
relay when it cannot, with the same code.

- One ALPN, `sourcelight/1`. A request opens a QUIC stream. Messages are serde
  types from `sl-protocol`; images follow their header as raw bytes.
- **No multicast from Rust on iOS.** Raw multicast needs an entitlement Apple
  has to approve. The QR code carries addresses, so the phone dials directly.
  Re-finding a moved desktop uses Bonjour through the native API, which does
  not need the entitlement.
- A plain HTTPS server on the desktop would be simpler for the same network
  alone. It is not chosen because the away-from-home step would then need a
  second, different transport.

### At home and away

Both are in version 1, with the same code. iroh tries a direct connection
first (same network, or through the router with hole punching) and falls back
to a relay. Most traffic ends up direct.

- **Development** uses n0's free public relays. They are rate-limited and "can
  change at any time", so they are not for the product.
- **Before launch**, run our own `iroh-relay` (open source, MIT/Apache-2.0) on
  a small server, about £5 a month, or pay n0 for hosted relays. The app ships
  with our relay's address; a setting can point it at another.
- With one relay of our own, the phone needs no lookup service: it stores the
  desktop's key and relay address at pairing and on every connection, and
  dials that.
- The relay cannot read the traffic, but it sees IP addresses and timing. Say
  so where away-from-home is switched on, and keep relay logs off. It is on by
  default once a phone is paired, with a switch in Settings → Phone.
- The desktop has to be on, awake and running SourceLight. That is the real
  limit of this design, and the phone should show when it last synced rather
  than let it be discovered.
- Someone with their own VPN to home can type the desktop's address by hand.

### What syncs

Measured on a real library of 1,117 photos: thumbnails (512 px) are about
30 KB each, previews (1600 px) about 209 KB.

| Data | Direction | 20,000 photos |
|---|---|---|
| Photo rows: time, size, rating, flag, label, tags, people, places, shoot, caption, focus band, burst group, keeper | desktop → phone | roughly 20 to 40 MB. Embeddings stay on the desktop. |
| Edit stacks of photos that are not raw | both ways, when a photo is opened or edited (section 4) | small, unless the stack has masks |
| Thumbnails | desktop → phone | about 600 MB for all. Fetched as needed, and in advance for chosen shoots and collections. |
| Previews | desktop → phone | about 4.2 GB for all, so never all. Fetched as needed and kept for shoots marked "Keep on phone". Cache limit, default 2 GB. |
| Decisions (rating, flag, label, tags) | phone → desktop | bytes |
| GPS tracks, phone photos | phone → desktop | as recorded |
| Exports | desktop → phone | as chosen |

**How changes move.**

- Desktop to phone: a change feed. Each photo row gets a sequence number,
  bumped by triggers when a synced field or its tags change; deleted photos
  leave a tombstone. The phone asks for "everything since N".
- Phone to desktop: operations, each with an id and a hybrid logical clock
  time, such as "set rating 4 on photo 812". The desktop applies them
  idempotently through the same code as its own commands, so its interface
  refreshes and its backups see them.
- Conflicts: last writer wins per field, by the clock time. There is one
  person on both devices, so conflicts are rare, and a later decision on
  either side wins. Tags are added and removed individually, never replaced as
  a list.
- **Catalogue identity.** The desktop gets a catalogue id (new) and an epoch
  that changes when a backup is restored. A different id or epoch makes the
  phone start a full resync rather than trust sequence numbers that went
  backwards.

### Licensing on the phone

- The phone app is free and open source under the same licence as the
  desktop, the Mozilla Public License 2.0 ([LICENSING.md](LICENSING.md)), and
  works with any SourceLight desktop. Nothing on either side is sold or
  unlocked, so there is no purchase flow and no licence check: the desktop's
  approval step (`sl_link::Approver`) only decides which phones are paired.
- "Send to phone" is an export like any other, with nothing extra in its way.
- Store listings say what the app needs: "Needs SourceLight on your Mac or PC".

### Desktop changes needed

1. A catalogue id and a restore epoch in `settings`.
2. A migration for the change feed: a sequence column on photos, triggers on
   photos and `photo_tags`, a tombstone table, and per-field change times for
   last-writer-wins.
3. A `paired_devices` table.
4. The link service: the iroh endpoint on the existing tokio runtime, with
   handlers that call the existing `db` functions.
5. Settings → Phone: pair (QR code), list, remove, on and off, and "Keep on
   phone" for shoots and collections.
6. An inbox: GPS tracks go to `gpx.rs` and are applied with a toast, phone
   photos added from the phone go through `ingest.rs` into a folder the user
   chose, with their edits (section 4).
7. An outbox: "Send to phone" as an export destination.

---

## 4. Editing on the phone

Decided 2026-10-03: edit non-raw photos on the phone itself, with the
phone's own photos as a second library kept apart from the synced one.

### Two libraries

- **On this phone**: the phone's own photos, the gallery and its albums. The
  app reads them where they are. It keeps a reference to each, its own
  thumbnails and any edits, never a copy. Nothing here leaves the phone unless
  the person chooses to send it.
- **Library**: the desktop's catalogue, synced as it is now.
- **Add to library**: select photos under On this phone and send them, with
  their edits, to the desktop. The desktop files the originals in a folder
  (Settings → Phone, default a `From phone` folder in the first library
  folder) and imports them through `ingest.rs` like any other photo, so a
  phone's GPS reaches Places. From then on the library copy owns the edits:
  the phone photo shows "In library" and points to the Library tab, so there
  are never two sets of edits drifting apart. The phone remembers which library
  photo each one became, so nothing is added twice; the desktop also skips a
  file it already has.

### What can be edited where

| Photo | On the phone | On the desktop |
|---|---|---|
| A phone photo: JPEG, HEIC, PNG, WebP | Edit, save a copy to the gallery | Once added |
| A library JPEG, PNG, TIFF, WebP or HEIC | Edit; edits sync both ways | Edit |
| A library raw | View and cull | Edit |
| A library photo whose edits use magic erase, AI denoise or a LUT | View | Edit |
| A phone raw (DNG) | View | Once added |

### How editing works

- **The desktop's own pipeline.** Desktop Develop already opens JPEG, PNG,
  TIFF, WebP and HEIC: it uploads the decoded pixels as the linear
  image the raw path would have made (`Pipeline::prepare_rendered`), and
  everything after that runs unchanged. The phone runs that path and the
  develop pass on its own GPU through wgpu (Vulkan on Android, Metal on iOS).
  Same code and shaders, so an edit looks the same on both.
- **Where the pixels come from.** A phone photo: the original, decoded on the
  phone. A library photo: its 1600 px preview, already cached, which for a
  JPEG is the same image scaled down. Fetching the original for a 100% view
  can come later.
- **Edits are settings**, the desktop's `EditStack` as JSON, kept in the
  phone's database with an undo history. The original is never written.
- **Frames.** The develop pass renders at screen size and the frame reaches
  the webview through a custom scheme, as thumbnails do, as a JPEG: on a
  Pixel 8 Pro raw pixels drawn into a canvas were slower (step 1).
- **Save to gallery** renders at full size on the phone and writes a new JPEG
  to Pictures/SourceLight through `mobile/plugins/gallery`.
- **Controls, first version**: light (exposure, contrast, highlights,
  shadows, whites, blacks), colour (white balance, vibrance, saturation), crop
  and straighten, looks and presets. Then curves, the colour mixer, grading,
  detail and effects. Masks and the AI tools stay on the desktop.

### Syncing edits

- Each row of the change feed has an edit tag, a short hash of the photo's
  stored edits, and where the photo can be edited. The phone fetches a
  photo's edits when it opens it, and keeps them while the tag still
  matches, not with every row, since a stack with masks can be large.
- An edit on the phone goes back as "set the edits of photo 812 to this,
  starting from that". If the photo's edits are still what the phone started
  from, the desktop applies it, through Develop's history when the photo is
  open there, so undo still works. If they changed on the desktop meanwhile,
  the phone's edit goes on a new virtual copy and the desktop's stays where
  it is: nothing is lost, and nothing is merged by guesswork.
- Settings the phone's app is too old to know survive its edit: the desktop
  lays the phone's edit over the one it started from.
- A stack the phone cannot render (magic erase, AI noise reduction, a LUT
  file only the desktop has) is marked as such in the feed. The phone shows
  the preview and says why it can't edit it. AI masks are kept in the edit
  itself, so the phone renders those.

### How it is built

- **`src-tauri/crates/sl-develop`**: the edit model (`develop/`), the GPU
  pipeline (`gpu/`), image decoding (`raster`) and the raw decode contract
  (`raw/`, without LibRaw), moved out of the desktop crate. The desktop
  re-exports them (`pub use sl_develop::{develop, gpu, raster, raw}`), so the
  rest of its code does not change. These modules use little outside
  themselves. The exceptions (the export encoder, the AI denoise helpers, the
  AI subject and SAM2 models, `exif_read`) either move with them or stay
  behind a desktop-only feature. Do the move after the magic erase PRs
  merge, since they change the same files.
- **A photos plugin** (`mobile/plugins/photos`): lists the gallery and its
  albums (Android MediaStore, iOS PhotoKit), makes thumbnails
  (`loadThumbnail`, `PHImageManager`), reads originals, and decodes HEIC and
  HEIF through the platform (Android `ImageDecoder`, iOS ImageIO), since the
  Rust decoders do not read them.
- **Permissions.** Android 13 and later ask for `READ_MEDIA_IMAGES`, and from
  Android 14 the person may allow only some photos: show those, with a button
  to choose more. Older Android asks for storage access. iOS has full or
  limited access. Google Play wants a declaration for broad photo access; a
  photo editor qualifies.
- **The phone's database** gains the phone photos (platform id, name, time,
  size, the library photo once added), their edits and undo history, and
  unsent edits to library photos.
- **Protocol** (`sl-protocol`): `AddPhoto { name, taken_ms, edits }` with the
  original as the body, answered with the new photo's id; `Edits { id }`,
  answered with the edits, their tag and where the photo can be edited;
  `SetEdits { id, edits, base }`, answered with the new tag or the copy
  made; and an edit tag and where it can be edited on each `PhotoRow`.

### Order of work, one PR each

1. **GPU test.** The desktop's Develop code built into the phone app (copied
   in at build time for now, not yet a crate) with a test screen: the GPU,
   and whether it meets the limits Develop asks for; how long the device and
   shaders take; and upload, render and frame times at screen size for a
   1600 px preview and a 12 MP photo, plus a live exposure slider. Run on
   a real phone. It decides whether the rest goes ahead as planned, and how
   frames reach the screen.

   Built 2026-10-03: `mobile/gpu-test`, behind the GPU test button in the
   phone app's library header. `mobile/gpu-test/examples/bench.rs` runs the
   same test on a computer. It copied the desktop's code in at build time
   until step 2; it uses `sl-develop` now. Found so far, on the Android
   emulator's software GPU:
   - The pipeline runs on Android and renders correctly, with the sliders
     working, as JPEG frames and as raw pixels drawn into a canvas.
   - Develop asks for 64 KB uniform buffers, and the emulator offers 16 KB,
     so its device request fails. Given the GPU's own limits the pipeline
     works, since its largest uniform block is about 1.3 KB. The shared crate
     should ask for no more than the GPU has where the pipeline does not need
     the full default.
   - In a debug build wgpu names every GPU object for debuggers, and that
     call crashes the emulator's Vulkan driver. The test uses a release
     build's flags.
   - On a desktop GPU a 1400 px frame renders in about 1 ms, but encoding it
     as a JPEG takes about 50 ms. Getting frames to the screen, not
     rendering, is the likely limit on a phone.

   On a Pixel 8 Pro (2026-10-04; Mali-G715, Vulkan; screen 1344 ×
   2991):
   - It meets every limit Develop asks for, with no fallback.
   - Getting the GPU ready takes 30 to 60 ms, and compiling the shaders 0.4
     to 0.55 s, once per run of the app. The app now does this while a photo
     sits in the loupe (`edit_warm`), so Edit doesn't wait for it.
   - Opening a photo: a 1600 px preview decodes and uploads in about 0.1 s,
     and a 12 MP photo in about 0.45 s.
   - A 1344 × 896 frame renders in 20 to 31 ms (worst 45 ms), and a 12 MP
     photo at full size in about 140 ms.
   - Encoding that frame as a JPEG takes 34 to 45 ms with image's encoder:
     more than rendering it. A live frame took about 100 ms end to end
     (render 29, encode 36, the rest getting it to the page). Sent as raw
     pixels it took longer, about 113 ms (render 34, the rest moving 4.8 MB),
     so frames stay JPEGs. While a slider moves the editor asks for half-size
     frames, a quarter of the pixels, and a sharp frame once it stops.
   - So frames are now encoded with `jpeg-encoder`, about four times quicker
     than image's on a desktop CPU without SIMD (10 ms against 39 ms for a
     1344 × 896 frame) and smaller. Frames keep colour at half resolution
     (4:2:0); a saved copy keeps it at full (4:4:4). Its licence adds the
     IJG's to MIT or Apache-2.0: IJG was allowed (2026-10-04), whose one
     condition, the credit line, is in the phone's notices.
2. **The `sl-develop` crate**, once magic erase has merged. The desktop does
   not change, and its render tests (`gpu/golden.rs`) show it.

   Done 2026-10-03: `src-tauri/crates/sl-develop` holds the edit
   model, the GPU pipeline, image decoding, the raw decode contract and
   EXIF reading. LibRaw, the AI masks and models, magic erase's model and
   fills, Develop's live state and the presenters stay in the desktop, which
   re-exports the crate's modules under their old names. The phone app and
   the GPU test depend on it directly.
3. **On this phone**: the photos plugin, permissions, albums, a grid with its
   own thumbnails, and a switch between the two libraries.

   Built 2026-10-03 (Android): `mobile/plugins/photos` (MediaStore; Android's
   own thumbnail cache for grid tiles, `ImageDecoder` for 2048 px loupe
   copies, so HEIC needs nothing of ours). Tiles and copies are cached in the
   app's cache folder, named by photo id and modified time, so an edited photo
   is made again, and served as `devthumb://` and `devpreview://`. A tab bar
   switches between Library and On this phone; each keeps its place. Limited
   access shows the photos chosen and a Choose more button, which brings back
   Android's picker. Photos the app saved itself (Send to phone) are always
   visible to it. Not yet: iOS, a RAW badge, editing.
4. **Develop on the phone**: the touch screen, edits and undo in the phone's
   database, Save to gallery.

   Built 2026-10-03 (Android), on `sl-develop` since step 2. Edit from a phone photo's loupe (not raws). Light,
   Colour (the desktop's labels and ranges), Crop (aspect presets through the
   desktop's `cropView.ts`, 90° turns, flip, straighten) and Looks (the
   desktop's built-in Styles). Frames come from `editframe://` with the whole
   edit in the name: half size while a slider moves, full once it stops. Hold
   the photo to see it unedited. A change is kept 0.6 s after the last one as
   one undo step, in `device.db` (`edits.rs`), apart from the library copy so
   a resync never touches it; reopening a photo brings its edits back.
   Photos over 4096 px, and HEIC or DNG, are decoded by Android to that size
   for editing; JPEG, PNG and WebP up to it are edited from their own bytes.
   Save copy renders at full size into Pictures/SourceLight as
   `<name>-edit.jpg`, with the original's EXIF (date, camera, location), so
   it sorts beside it. Photo access now also asks for
   `ACCESS_MEDIA_LOCATION`, which Android grants with it, without which it
   blanks every photo's location for the app.
5. **Add to library**: the upload, the import on the desktop, the link
   between the two copies, repeats skipped.

   Built 2026-10-03 (Android). With a computer paired: Select in On this
   phone, or Add to library in a photo's loupe. Chosen photos wait in
   `device.db` and go on the next sync, each as stored (location included,
   `setRequireOriginal`) with its edits (`Request::AddPhoto`). The desktop
   files it in the import folder (Settings → Phone: default a `From phone`
   folder in the first library folder, else Pictures/SourceLight), scans
   that folder as any library folder (thumbnails, EXIF, places), files it
   under the library folder it is in, applies the edits, and keeps the
   bytes' SHA-256 (migration 51) so the same file sent again finds its
   photo. The phone marks it Waiting, then In library; its loupe then says
   it is in the library rather than offering Edit, since the library copy
   owns the edits. A photo that can't be added is marked Not added and
   waits for the person to try again.
6. **Editing library photos on the phone**: edit versions in the feed,
   stacks fetched and sent, the desktop applying them through its history.

   Built 2026-10-03 (Android). Each feed row has an edit tag and where the
   photo can be edited: here too (a JPEG, PNG, TIFF, WebP or HEIC whose
   edits need nothing outside themselves), a raw, or the computer only.
   Migration 52 moves a photo up the feed when its edits really change, and
   every photo once, so phones learn this; a phone that updates later
   fetches the whole library again once for the same reason. Edit in a
   library photo's loupe opens its cached preview with its edits: an edit
   not yet sent, else the computer's from last time while the tag matches,
   else fetched (`Request::Edits`). Each change goes to the computer
   straight away when connected, otherwise with the next sync
   (`Request::SetEdits`, with the edits it started from). The desktop puts
   it on the photo, through Develop when the photo is open there (an undo
   step called "Edited on phone"); if the photo was edited on the computer
   since, the phone's edit goes on a new virtual copy named "From phone" and
   the phone says so. Tiles say Edited. Raws, and photos only the computer
   can show, say why instead of opening. Once an edit has gone on a copy,
   the rest of that editing goes to the same copy, and the editor says so.
   Not yet: the loupe shows photos unedited, as the desktop's grid does;
   editing from the original for a 100% view; the built-in film looks, which
   are LUT files on the desktop. Found on the way: the phone's batched UDP
   sends stalled multi-packet requests (see Platform gotchas).
7. **iOS**: PhotoKit, HEIC and Metal. Needs Xcode on the Mac mini.

   Built 2026-10-05 (iOS simulator, iOS 27). Pairing by pasting the link,
   the library, culling and editing work as on Android, the editor on Metal.
   The plugins have iOS sides (`ios/` in each, Swift):
   - **photos**: PhotoKit. All photos or the chosen ones ("Choose more"
     opens iOS's picker), the person's albums and the useful smart albums,
     JPEGs through PhotoKit and ImageIO, so HEIC needs nothing of ours. An
     HEIC (10 bits) is decoded to 8 bits in its own colour space (Display
     P3 on an iPhone) for editing. PhotoKit names a photo by a string; the
     app keys photos by number, so each is numbered by a 53-bit hash of its
     identifier and mapped back. Raws are the RAW smart album's.
   - **gallery**: saves into Photos with add-only access. No album, which
     would need access to read the library too.
   - **shoot-track**: CoreLocation with "while using", carrying on in the
     background (the `location` background mode, with the blue indicator).
     iOS has no interval, so a fix is kept every `intervalMs`, or sooner when
     much sharper. If iOS ends the app, the recording ends with it.
   - **qr-scan** is not done: the simulator has no camera to test it with,
     so Scan the code stays Android only.

   On an iPad (checked on the iPad Air 11-inch simulator): the grid and the
   loupe sit side by side in landscape (900 px and wider), one pane in
   portrait. The editor puts its controls beside the photo in landscape
   (the `beside` variant in `mobile/src/styles.css`: a phone on its side, or
   a tablet in landscape), so the photo is not a strip between black bars.
   Pairing names it "iPad", and the form keeps a readable width.

### Risks

- **Phone GPUs.** Develop asks wgpu for its default limits, a desktop-class
  floor. Older or cheaper Android GPUs may fall short. If so, find which
  limit, and whether the pipeline can manage with less or the app should ask
  for a recent phone. The GPU test answers this: a Pixel 8 Pro meets every
  limit. Older and cheaper phones are still to try.
- **Memory.** A 12 MP photo is about 100 MB on the GPU as the pipeline's
  half-float image and its mip levels. Fine on current phones. A 48 to
  200 MP phone photo is not, so develop those at a reduced size, which the
  pipeline already does for images past the GPU's texture limit.
- **Getting frames on screen.** A webview cannot show a GPU texture directly,
  so each frame is read back and handed over. If that makes sliders stutter,
  the fallback is a native surface under the webview, as the desktop's direct
  presenter does.
- **Detail at preview size.** Sharpening and noise reduction look different
  on a 1600 px preview than on the full photo, as they do on the desktop's
  1400 px proxy.

---

## Building the phone app

The app is in `mobile/`: `mobile/src` is the interface (it imports the
desktop's styles, translations and logic from `../src`), `mobile/src-tauri` is
its Rust core. `mobile/package.json` has scripts only, so that npm run from
inside `mobile/` (as Android's Gradle build does) finds the phone app rather
than the desktop; the packages are the repo root's. It is its own Cargo project, with its own `Cargo.lock` and
`target/`, and it depends on `src-tauri/crates/sl-link` by path. Its app id is
`app.sourcelight.mobile`, distinct from the desktop's `app.sourcelight`, so
the two stay separate apps in each store.

**Android, on Windows.** Needs Android Studio's SDK with NDK r30
(`ndk;30.0.16248370`), platform 36, and Rust's Android targets
(`rustup target add aarch64-linux-android armv7-linux-androideabi
i686-linux-android x86_64-linux-android`). In the shell:

```bash
export JAVA_HOME="C:/Program Files/Android/Android Studio/jbr"
export ANDROID_HOME="$LOCALAPPDATA/Android/Sdk"
export NDK_HOME="$ANDROID_HOME/ndk/30.0.16248370"
npm run android:build   # a debug APK in mobile/src-tauri/gen/android/app/build/outputs/apk
npm run android:phone   # the same, for 64-bit ARM phones only (any phone from the last
                        # several years): smaller to copy over
npm run android:dev     # live reload on a connected phone or the emulator
```

Install the APK on a phone with `adb install -r <apk>` (USB debugging on), or
copy it to the phone and open it.

**iOS, on the Mac.** Needs Xcode with an iOS simulator runtime, Rust's iOS
targets and `llvm-tools`, and xcodegen and CocoaPods from Homebrew (the
CocoaPods gem wants sudo):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
rustup component add llvm-tools
brew install xcodegen cocoapods
```

The Xcode project is committed in `mobile/src-tauri/gen/apple`. From
`mobile/`, `npx tauri ios build --debug --target aarch64-sim` builds for the
simulator (about 7 minutes from cold; the app is in Xcode's DerivedData, under
`Build/Products/debug-iphonesimulator/SourceLight.app`), and `npx tauri ios
dev` runs it with live reload. A real iPhone needs an Apple Developer account
to sign the build.

- Xcode 27 makes the Swift side's exports local. swift-rs promotes them again
  with `llvm-objcopy` (hence `llvm-tools`), but missed its own runtime's, so
  `mobile/src-tauri/vendor/swift-rs` is a patched copy
  (`vendor/swift-rs/VENDORED.md`).
- `bundle.iOS.frameworks` in `tauri.conf.json` links what Swift's
  auto-linking can't be trusted to: SystemConfiguration (iroh), Photos,
  PhotosUI and CoreLocation. PhotosUI above all: its picker is a category
  on `PHPhotoLibrary`, so no symbol pulls it in, and without it "Choose
  more" crashed with an unrecognised selector. Changing that list means
  regenerating the project:
  delete `gen/apple` and run `npx tauri ios init --ci`, then make the launch
  screen's background `#0c0c0e` again in `LaunchScreen.storyboard`.
- `mobile/src-tauri/Info.ios.plist` is merged into the app's Info.plist: the
  local network prompt's text.
- `Info.ios.plist` also has the photo library, add-to-Photos and location
  prompts' texts, and the `location` background mode for GPS tracks.
- Each plugin's Swift package is built by its Rust build script (Tauri's
  plugin build, through swift-rs), which copies Tauri's Swift API into
  `ios/.tauri` (ignored). `swift-tools-version` is 5.5, for `.iOS(.v15)`:
  Xcode 27 refuses older deployment targets.
- The QR scanner has no iOS side yet, so Scan the code shows on Android
  only and an iPhone pairs by pasting the link.
- Test photos: `xcrun simctl addmedia <device> <files>`; a route for GPS
  tracks: `xcrun simctl location <device> start --speed=3 <lat,lon> …`.

## Platform gotchas

- **iOS stops background networking.** Sync runs while the app is open. "Open
  the app to sync" is how it works under any framework, so design for it.
- **iOS local network prompt.** `NSLocalNetworkUsageDescription` and the
  Bonjour service type must be declared. If the user says no, the link fails
  silently unless the app detects it and explains how to fix it.
- **Android 17 local network permission.** Apps targeting API 37 need the
  runtime `ACCESS_LOCAL_NETWORK` permission for any LAN traffic. Play will
  require that target within a year of its release, so build for it from the
  start.
- **Background location review.** iOS needs the `location` background mode and
  a clear reason. On Android, start tracking from a foreground service while
  the app is visible. That needs only fine location, not background location,
  which has a much stricter Play review. Play still asks for a declaration of
  the foreground service type.
- **Windows Firewall** asks once when the desktop first listens. Ask for phone
  connections in a way that makes the prompt expected. Check whether the Store (MSIX)
  build needs a network capability declared for incoming connections.
- **App Review has no desktop.** Apple needs a demo mode: a small bundled
  library the app opens without pairing. The same demo mode serves screenshots
  and testers.
- **Google Play's new-account rule.** Personal developer accounts created after
  13 November 2023 must run a closed test with 12 testers for 14 continuous
  days before production. The 0.1.0 friends could be those testers, using the
  demo mode if they have no desktop install.
- **Gradle dependencies are not licence-checked.** `cargo deny` covers Rust
  crates only. The official `tauri-plugin-barcode-scanner` brought in
  Google's closed-source ML Kit unnoticed, so it was replaced (2026-10-03)
  with `mobile/plugins/qr-scan` on ZXing (Apache-2.0). The track plugin
  uses Android's own `LocationManager`, not Google Play services. Check the
  licence of anything added to a plugin's `build.gradle.kts` by hand.
- **Desktop asleep or closed.** Nothing syncs. The phone says it can't
  reach the computer, and when it last synced.
- **Batched UDP sends from the phone.** QUIC sends several packets in one
  call where the system allows it (segmentation offload). From the Android
  emulator, a request of more than one packet reached the desktop as its
  first packet only; the rest came seconds to a minute later, when other
  traffic happened to move the connection on, and everything queued behind
  it waited. One-packet requests were never affected, which hid it until
  edits (a couple of KB each) went phone to desktop. The phone's endpoint now
  sends one packet at a time (`sl-link` `bind`), and the stalls went (round
  trips of a few ms). The emulator's virtual network is the likely culprit,
  but some phone drivers, VPNs and hotspots mangle batches too, and a phone
  sends too little for batching to matter. Every phone request also gives up
  after 30 s without moving a byte (`STALL`, two minutes more while a pairing
  waits for the person at the desktop), so a connection that stops carrying
  data is dropped and made again instead of holding up syncing until the app
  restarts.

## Costs

| Item | Cost | Note |
|---|---|---|
| Apple Developer Program | $99 a year | Needed anyway to notarise the Mac app. |
| Google Play developer account | $25 once | |
| Our relay | about £5 a month | From launch. Free public relays during development. |
| Test devices | a mid-range Android | If none is to hand. |

## Order of work

Rough, at 10 to 15 hours a week. Starts after the desktop's 1.0.

| Step | What | Weeks |
|---|---|---|
| 0 | The three spikes | 3 |
| 1 | Link: workspace crates, pairing, relay, catalogue id and epoch, change feed, Settings → Phone, a phone shell that pairs and lists shoots | 4 to 5 |
| 2 | Browse, cull, offline cache, operations syncing back, iPad layout | 7 to 9 |
| 3 | Shoot track and Send to phone | 3 to 4 |
| 4 | Our relay server; store work: demo mode, privacy labels, TestFlight, the 14-day Play test | 3 to 4, plus waiting |
| 5 | Editing on the phone: the seven steps in section 4 | not yet estimated; the GPU test comes first |
| Then | 1.x: search, people queue, clock check | |

## Decisions and what is still open

Answered on 2026-10-02:

- Remote access is required. It is in version 1, through our own relay.
- The iPad layout is in version 1.
- Both stores use the name SourceLight for now.
- There is no Play developer account yet. A new personal account means the
  12-tester, 14-day closed test. Deal with it near release.
- How to charge was left open. It has since been settled: SourceLight is open
  source and nothing is sold (see "Licensing on the phone").

Answered on 2026-10-03:

- Edit photos that are not raw on the phone, with the phone's own photos as a
  second library kept apart from the synced one, and chosen photos added to
  the synced library (section 4).
