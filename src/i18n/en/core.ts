/**
 * English strings for the core area: the sentences the Rust core sends as text
 * (command errors, AI readiness, job messages, model purposes), worded here so
 * the interface can translate them. `lib/coreText.ts` matches the core's
 * English against these values, so each `core.msg.*` and `core.progress.*`
 * value must stay EXACTLY the core's wording, `{placeholders}` standing where
 * the core formats a value in; `lib/coreText.test.ts` fails when the Rust source
 * no longer contains them. `core.model.<id>.purpose` is keyed by the model's id.
 */
export default {
  "core.msg.downloadBroken": "The download of {what} didn't arrive intact — try again",
  "core.msg.downloadShort": "The download of {what} stopped before the end — try again",

  // --- Folders and files -------------------------------------------------
  "core.msg.notFolderDrop": "{name} isn't a folder — drop the folder your photos are in",
  "core.msg.notFolder": "{path} isn't a folder",
  "core.msg.notFolderDir": "{directory} is not a folder",
  "core.msg.folderInside": "{path} is already in the library, inside {parent}",
  "core.msg.folderOverlaps": "{path} overlaps {other}, which is already in the library",
  "core.msg.fileGone": "{path} isn't there any more",
  "core.msg.noDrag": "none of these photos' files are there to drag",
  "core.msg.pathNoFilename": "that path has no filename",
  "core.msg.cannotWrite": "cannot write {path}: {reason}",
  "core.msg.cannotCreate": "cannot create {path}",
  "core.msg.cannotRemove": "couldn't remove {path}",
  "core.msg.cannotRead": "cannot read {path}: {reason}",
  "core.msg.couldntRead": "Couldn't read {path}",
  "core.msg.gpxUnreadable": "Not a readable GPX file: {reason}",
  "core.msg.noSpace":
    "Not enough free space for {what}: it needs about {needed}, and the drive holding {folder} has {free} free.",
  "core.msg.heicNeedsStore":
    "Reading HEIC needs two free Microsoft Store apps: HEIF Image Extensions and HEVC Video Extensions",
  "core.msg.heicNotRead": "HEIC is not read on this system yet: {path}",

  // --- Library records ---------------------------------------------------
  "core.msg.placeNeedsName": "a place needs a name",
  "core.msg.collectionNeedsName": "a collection needs a name",
  "core.msg.smartCollectionNeedsName": "a smart collection needs a name",
  "core.msg.presetNeedsName": "a preset needs a name",
  "core.msg.personNeedsName": "a person needs a name",
  "core.msg.faceNotCover": "that face can't be a cover",
  "core.msg.faceGone": "that face is no longer in the library",
  "core.msg.faceTaken": "that face is already someone's — take it out with “Not them” first",
  "core.msg.personGone": "that person is no longer in the library",
  "core.msg.photosGone": "none of those photos are in the library",
  "core.msg.photoNotFound": "photo {id} not found",
  "core.msg.presetNotFound": "preset {id} not found",
  "core.msg.noPhotoOpen": "no photo is open",
  "core.msg.noBackup": "there is no backup that passes its integrity check",
  "core.msg.backupGone": "that backup no longer exists: {path}",
  "core.msg.schemaNewer":
    "library database is schema v{current}, but this build only understands up to v{target} — update the app",
  "core.msg.updatesNotSetUp": "Updates are not set up for this build.",
  "core.msg.newestVersion": "You already have the newest version.",

  // --- Develop -----------------------------------------------------------
  "core.msg.lutOnlyCube": "only .cube LUTs are supported",
  "core.msg.lutBadName": "invalid LUT name",
  "core.msg.lut1d": "1D LUTs are not supported — this stage expects a 3D (LUT_3D_SIZE) .cube",
  "core.msg.lutNoSize": "no LUT_3D_SIZE found — not a 3D .cube",
  "core.msg.tooDark": "That spot is too dark to read. Pick something grey and well lit.",
  "core.msg.selectionLost": "the selection model lost its place — click again",
  "core.msg.denoiseUnavailable": "AI noise reduction is not available",
  "core.msg.denoiseFailed": "AI noise reduction: {reason}",
  "core.msg.denoiseClosed": "the photo was closed before noise reduction finished",

  // --- Builds without a feature ------------------------------------------
  "core.msg.noRaw": "this build was compiled without raw decoding",
  "core.msg.noAiFeatures": "This build was compiled without AI features.",
  "core.msg.noAiDenoise": "This edit uses AI noise reduction, which this build does not include.",
  "core.msg.noAiRebuild": "this build was compiled without AI — rebuild with `--features ai`",
  "core.msg.noAi": "this build was compiled without AI",
  "core.msg.noAiSupport": "This build was compiled without AI support",
  "core.msg.noSemantic": "semantic search needs the AI-enabled build",
  "core.msg.noTagger": "this build has no built-in tagger — rebuild with `--features ai`",

  // --- AI models and runtime ---------------------------------------------
  "core.msg.alreadyDownloading": "'{id}' is already downloading",
  "core.msg.unknownModel": "unknown model '{id}'",
  "core.msg.visionFailedLoad": "the {model} vision model failed to load",
  "core.msg.visionSlow": "the {model} vision model took too long to load",
  "core.msg.visionNotDownloaded": "the {model} vision model isn't downloaded",
  "core.msg.visionNotSupported":
    "{model} isn't one of the vision models SourceLight can use — choose one from the list",
  "core.msg.visionNotLoaded": "the vision model is not loaded",
  "core.msg.visionStillLoading": "the vision model is still loading",
  "core.msg.emptyCaption": "the model returned an empty caption",
  "core.msg.noDescriptions":
    "this tagger can't write descriptions — turn on Detailed descriptions in Settings → AI",
  "core.msg.runtimeNotInstalled": "the local AI runtime isn't installed",
  "core.msg.searchModelFailed": "the search model failed to load; try re-downloading it in Settings",
  "core.msg.faceModelsFailed": "the face models failed to load; try re-downloading them in Settings",
  "core.msg.notIndexed": "this photo isn't indexed for search yet — run indexing first",

  // --- AI readiness (the status pill and the setup dialog) ---------------
  "core.msg.readyPurpose": "Ready · {purpose} runs on this machine",
  "core.msg.readyBuiltinTagging": "Ready · built-in tagging, runs on this machine",
  "core.msg.readyModel": "Ready · {model}",
  "core.msg.readyBuiltin": "Ready · built-in, runs locally",
  "core.msg.aiSwitchedOff": "On-device AI is switched off — turn it back on in Settings → AI",
  "core.msg.descriptionsNeedVision":
    "Descriptions need the vision model — turn on Detailed descriptions in Settings → AI",
  "core.msg.statusNotSetUp": "Not set up yet — this happens by itself the first time you use it",
  "core.msg.statusAiOff": "AI is off — tags are yours to write, or turn it back on in Settings → AI",
  "core.msg.statusNoTagger": "This build has no built-in tagger (compiled without AI)",
  "core.msg.statusTagModel": "The tagging model downloads the first time you tag or search",
  "core.msg.statusModelDownloads": "{model} downloads the first time descriptions are needed",
  // What has to be downloaded first, as the setup dialog lists it.
  "core.msg.reqModel": "the {name} model",
  "core.msg.reqVision": "the {model} vision model",
  "core.msg.reqRuntime": "the local AI runtime",
  // What an AI feature is called mid-sentence ("Ready · tagging runs on this machine").
  "core.msg.termTagging": "tagging",
  "core.msg.termCaptions": "captions",
  "core.msg.termSearch": "photo search",
  "core.msg.termFaces": "face detection",
  "core.msg.termPeople": "recognising people",
  "core.msg.termAnalysis": "photo search and people",
  "core.msg.termSubject": "subject selection",
  "core.msg.termClick": "click-to-select",
  "core.msg.termDenoise": "AI noise reduction",
  "core.msg.termImport": "the import",

  // --- What a job says it is doing ---------------------------------------
  "core.progress.importing": "Importing",
  "core.progress.readingLocations": "Reading locations",
  "core.progress.checkingColours": "Checking colours",
  "core.progress.loadingModel": "Loading model…",
  "core.progress.tagging": "Tagging",
  "core.progress.finished": "Finished",
  "core.progress.reducingNoise": "Reducing noise",
  "core.progress.indexing": "Indexing for search",
  "core.progress.stopped": "Stopped",
  "core.progress.indexed": "Indexed",
  "core.progress.analysingFaces": "Analysing faces",
  "core.progress.readingEyes": "Reading eyes",
  "core.progress.facesAnalysed": "Faces analysed",
  "core.progress.scoringFocus": "Scoring focus",
  "core.progress.scored": "Scored",
  "core.progress.captioning": "Captioning",
  "core.progress.groupingPeople": "Grouping people",
  "core.progress.readingFiles": "Reading files",
  "core.progress.indexingFiles": "Indexing",

  // --- What each on-device model is for (by model id) ---------------------
  "core.model.subject.purpose": "One-click subject / background cut-out",
  "core.model.click.purpose": "Interactive point-and-click masking (SAM 2)",
  "core.model.denoise.purpose":
    "Clears colour speckle and blotchy high-ISO noise in Develop, keeping edges (NAFNet)",
  "core.model.tagger.purpose":
    "Search by what photos show, in your own language, and tag them (SigLIP 2)",
  "core.model.faces.purpose": "Find faces, and flag closed eyes and soft faces in a cull",
  "core.model.people.purpose": "Recognise the same person across photos, to group them in People",
  "core.model.erase.purpose":
    "Takes people and objects out of a photo in Develop, filling in what was behind them (LaMa)",
  // Each model's name, by id, as the feature it serves is named in the app.
  "core.model.subject.name": "Select subject",
  "core.model.click.name": "Click to select",
  "core.model.denoise.name": "AI noise reduction",
  "core.model.erase.name": "Erase",
  "core.model.tagger.name": "Search and tagging",
  "core.model.faces.name": "Face tools",
  "core.model.people.name": "People",

  // Failures the core classified by their root cause (src-tauri/src/errors.rs,
  // lib/coreText.ts). {file} is a file name alone, never a path.
  "core.sys.notFound": "The file isn't there any more — it may have been moved, renamed or deleted.",
  "core.sys.notFoundFile": "{file} isn't there any more — it may have been moved, renamed or deleted.",
  "core.sys.accessDenied": "SourceLight isn't allowed to use this file or folder. Check its permissions.",
  "core.sys.accessDeniedFile": "SourceLight isn't allowed to use {file}. Check its permissions.",
  "core.sys.fileInUse": "Another program is using the file. Close it there and try again.",
  "core.sys.fileInUseFile": "Another program is using {file}. Close it there and try again.",
  "core.sys.readOnly": "The drive or file is read-only.",
  "core.sys.readOnlyFile": "{file} is on a read-only drive, or is read-only itself.",
  "core.sys.alreadyExists": "A file with that name is already there.",
  "core.sys.alreadyExistsFile": "{file} is already there.",
  "core.sys.fileDamaged": "The file is damaged or incomplete.",
  "core.sys.fileDamagedFile": "{file} is damaged or incomplete.",
  "core.sys.rawUnsupported": "This raw file isn't one SourceLight can read yet.",
  "core.sys.rawUnsupportedFile": "{file} isn't a raw file SourceLight can read yet.",
  "core.sys.imageUnsupported": "This image format isn't supported.",
  "core.sys.imageUnsupportedFile": "{file} is in a format SourceLight can't read.",
  "core.sys.diskFull": "The drive is full. Free some space and try again.",
  "core.sys.driveMissing": "The drive isn't connected.",
  "core.sys.outOfMemory": "The computer ran out of memory. Close other programs and try again.",
  "core.sys.catalogueBusy": "The catalogue is busy — another task is using it. Try again in a moment.",
  "core.sys.catalogueDamaged": "The catalogue is damaged. Restore a backup from Settings › Library.",
  "core.sys.catalogueReadOnly": "The catalogue can't be written to — its folder or drive is read-only.",
  "core.sys.catalogueCannotOpen": "The catalogue couldn't be opened.",
  "core.sys.catalogue": "The catalogue couldn't complete that.",
  "core.sys.offline": "Couldn't connect. Check the internet connection and try again.",
  "core.sys.timedOut": "The connection took too long. Try again.",
  "core.sys.httpStatus": "The server couldn't provide it (error {status}). Try again later.",
  "core.sys.network": "A network error stopped it. Try again.",
  "core.sys.aiRuntime": "The AI model failed to run.",
  "core.sys.raw": "The raw file couldn't be read.",
  "core.sys.rawFile": "{file} couldn't be read.",
  "core.sys.image": "The image couldn't be read or written.",
  "core.sys.imageFile": "{file} couldn't be read or written.",
  "core.sys.io": "A file couldn't be read or written.",
  "core.sys.ioFile": "{file} couldn't be read or written.",
  // "{context}" is the core's own sentence for what it was doing, translated.
  "core.sys.withContext": "{context}: {cause}",
} as const;
