import type { Catalogue } from "..";

// U+202F (narrow no-break space) stands before : ; ! ? and U+00A0 inside « ».
export default {
  "core.msg.downloadBroken": "Le téléchargement de {what} n’est pas arrivé intact — réessayez",
  "core.msg.downloadShort": "Le téléchargement de {what} s’est arrêté avant la fin — réessayez",

  // --- Folders and files -------------------------------------------------
  "core.msg.notFolderDrop": "{name} n’est pas un dossier : déposez le dossier qui contient vos photos",
  "core.msg.notFolder": "{path} n’est pas un dossier",
  "core.msg.notFolderDir": "{directory} n’est pas un dossier",
  "core.msg.folderInside": "{path} est déjà dans la bibliothèque, dans {parent}",
  "core.msg.folderOverlaps": "{path} chevauche {other}, qui est déjà dans la bibliothèque",
  "core.msg.fileGone": "{path} n’existe plus",
  "core.msg.noDrag": "Les fichiers de ces photos sont introuvables, impossible de les faire glisser",
  "core.msg.pathNoFilename": "Ce chemin n’a pas de nom de fichier",
  "core.msg.cannotWrite": "Impossible d’écrire {path} : {reason}",
  "core.msg.cannotCreate": "Impossible de créer {path}",
  "core.msg.cannotRemove": "Impossible de supprimer {path}",
  "core.msg.cannotRead": "Impossible de lire {path} : {reason}",
  "core.msg.couldntRead": "Impossible de lire {path}",
  "core.msg.gpxUnreadable": "Fichier GPX illisible : {reason}",
  "core.msg.noSpace":
    "Pas assez d’espace libre pour {what} : il en faut environ {needed}, et le disque qui contient {folder} n’a que {free} de libre.",
  "core.msg.heicNeedsStore":
    "La lecture du HEIC nécessite deux applications gratuites du Microsoft Store : Extensions d’image HEIF et Extensions vidéo HEVC",
  "core.msg.heicNotRead": "Le HEIC n’est pas encore lu sur ce système : {path}",

  // --- Library records ---------------------------------------------------
  "core.msg.placeNeedsName": "Un lieu doit avoir un nom",
  "core.msg.collectionNeedsName": "Une collection doit avoir un nom",
  "core.msg.smartCollectionNeedsName": "Une collection dynamique doit avoir un nom",
  "core.msg.presetNeedsName": "Un préréglage doit avoir un nom",
  "core.msg.personNeedsName": "Une personne doit avoir un nom",
  "core.msg.faceNotCover": "Ce visage ne peut pas servir de couverture",
  "core.msg.faceGone": "Ce visage n’est plus dans la bibliothèque",
  "core.msg.faceTaken":
    "Ce visage appartient déjà à quelqu’un : retirez-le d’abord avec « Pas cette personne »",
  "core.msg.personGone": "Cette personne n’est plus dans la bibliothèque",
  "core.msg.photosGone": "Aucune de ces photos n’est dans la bibliothèque",
  "core.msg.photoNotFound": "Photo {id} introuvable",
  "core.msg.presetNotFound": "Préréglage {id} introuvable",
  "core.msg.noPhotoOpen": "Aucune photo n’est ouverte",
  "core.msg.noBackup": "Aucune sauvegarde ne passe le contrôle d’intégrité",
  "core.msg.backupGone": "Cette sauvegarde n’existe plus : {path}",
  "core.msg.schemaNewer":
    "La base de données de la bibliothèque est au schéma v{current}, mais cette version ne comprend que jusqu’à v{target} : mettez l’app à jour",
  "core.msg.updatesNotSetUp": "Les mises à jour ne sont pas configurées pour cette version.",
  "core.msg.newestVersion": "Vous avez déjà la version la plus récente.",

  // --- Develop -----------------------------------------------------------
  "core.msg.lutOnlyCube": "Seules les LUT .cube sont prises en charge",
  "core.msg.lutBadName": "Nom de LUT non valide",
  "core.msg.lut1d": "Les LUT 1D ne sont pas prises en charge : cette étape attend un .cube 3D (LUT_3D_SIZE)",
  "core.msg.lutNoSize": "LUT_3D_SIZE introuvable : ce n’est pas un .cube 3D",
  "core.msg.tooDark": "Cet endroit est trop sombre pour être lu. Choisissez quelque chose de gris et bien éclairé.",
  "core.msg.selectionLost": "Le modèle de sélection a perdu le fil : cliquez à nouveau",
  "core.msg.denoiseUnavailable": "La réduction du bruit par IA n’est pas disponible",
  "core.msg.denoiseFailed": "Réduction du bruit par IA : {reason}",
  "core.msg.denoiseClosed": "La photo a été fermée avant la fin de la réduction du bruit",

  // --- Builds without a feature ------------------------------------------
  "core.msg.noRaw": "Cette version a été compilée sans décodage RAW",
  "core.msg.noAiFeatures": "Cette version a été compilée sans les fonctions d’IA.",
  "core.msg.noAiDenoise": "Cette retouche utilise la réduction du bruit par IA, que cette version n’inclut pas.",
  "core.msg.noAiRebuild": "Cette version a été compilée sans IA : recompilez avec `--features ai`",
  "core.msg.noAi": "Cette version a été compilée sans IA",
  "core.msg.noAiSupport": "Cette version a été compilée sans prise en charge de l’IA",
  "core.msg.noSemantic": "La recherche sémantique nécessite la version avec IA",
  "core.msg.noTagger": "Cette version n’a pas d’étiqueteur intégré : recompilez avec `--features ai`",

  // --- AI models and runtime ---------------------------------------------
  "core.msg.alreadyDownloading": "« {id} » est déjà en cours de téléchargement",
  "core.msg.unknownModel": "Modèle inconnu « {id} »",
  "core.msg.visionFailedLoad": "Le modèle de vision {model} n’a pas pu être chargé",
  "core.msg.visionSlow": "Le modèle de vision {model} a mis trop de temps à se charger",
  "core.msg.visionNotDownloaded": "Le modèle de vision {model} n’est pas téléchargé",
  "core.msg.visionNotSupported":
    "{model} ne fait pas partie des modèles de vision que SourceLight peut utiliser : choisissez-en un dans la liste",
  "core.msg.visionNotLoaded": "Le modèle de vision n’est pas chargé",
  "core.msg.visionStillLoading": "Le modèle de vision est encore en cours de chargement",
  "core.msg.emptyCaption": "Le modèle a renvoyé une légende vide",
  "core.msg.noDescriptions":
    "Cet étiqueteur ne sait pas écrire de descriptions : activez les descriptions détaillées dans Réglages → IA",
  "core.msg.runtimeNotInstalled": "Le moteur d’IA local n’est pas installé",
  "core.msg.searchModelFailed":
    "Le modèle de recherche n’a pas pu être chargé ; essayez de le télécharger à nouveau dans les Réglages",
  "core.msg.faceModelsFailed":
    "Les modèles de visages n’ont pas pu être chargés ; essayez de les télécharger à nouveau dans les Réglages",
  "core.msg.notIndexed": "Cette photo n’est pas encore indexée pour la recherche : lancez d’abord l’indexation",

  // --- AI readiness (the status pill and the setup dialog) ---------------
  "core.msg.readyPurpose": "Prêt · {purpose} fonctionne sur cet ordinateur",
  "core.msg.readyBuiltinTagging": "Prêt · étiquetage intégré, fonctionne sur cet ordinateur",
  "core.msg.readyModel": "Prêt · {model}",
  "core.msg.readyBuiltin": "Prêt · intégré, fonctionne en local",
  "core.msg.aiSwitchedOff": "L’IA sur l’appareil est désactivée : réactivez-la dans Réglages → IA",
  "core.msg.descriptionsNeedVision":
    "Les descriptions nécessitent le modèle de vision : activez les descriptions détaillées dans Réglages → IA",
  "core.msg.statusNotSetUp": "Pas encore configuré : cela se fait tout seul à la première utilisation",
  "core.msg.statusAiOff":
    "L’IA est désactivée : les mots-clés sont à écrire vous-même, ou réactivez-la dans Réglages → IA",
  "core.msg.statusNoTagger": "Cette version n’a pas d’étiqueteur intégré (compilée sans IA)",
  "core.msg.statusTagModel": "Le modèle d’étiquetage se télécharge la première fois que vous étiquetez ou recherchez",
  "core.msg.statusModelDownloads": "{model} se télécharge la première fois que des descriptions sont nécessaires",
  // What has to be downloaded first, as the setup dialog lists it.
  "core.msg.reqModel": "le modèle {name}",
  "core.msg.reqVision": "le modèle de vision {model}",
  "core.msg.reqRuntime": "le moteur d’IA local",
  // What an AI feature is called mid-sentence ("Ready · tagging runs on this machine").
  "core.msg.termTagging": "l’étiquetage",
  "core.msg.termCaptions": "les légendes",
  "core.msg.termSearch": "la recherche de photos",
  "core.msg.termFaces": "la détection des visages",
  "core.msg.termPeople": "la reconnaissance des personnes",
  "core.msg.termAnalysis": "la recherche de photos et les personnes",
  "core.msg.termSubject": "la sélection du sujet",
  "core.msg.termClick": "la sélection en un clic",
  "core.msg.termDenoise": "la réduction du bruit par IA",
  "core.msg.termImport": "l’importation",

  // --- What a job says it is doing ---------------------------------------
  "core.progress.importing": "Importation",
  "core.progress.readingLocations": "Lecture des lieux",
  "core.progress.checkingColours": "Vérification des couleurs",
  "core.progress.loadingModel": "Chargement du modèle…",
  "core.progress.tagging": "Étiquetage",
  "core.progress.finished": "Terminé",
  "core.progress.reducingNoise": "Réduction du bruit",
  "core.progress.indexing": "Indexation pour la recherche",
  "core.progress.stopped": "Arrêté",
  "core.progress.indexed": "Indexé",
  "core.progress.analysingFaces": "Analyse des visages",
  "core.progress.readingEyes": "Lecture des yeux",
  "core.progress.facesAnalysed": "Visages analysés",
  "core.progress.scoringFocus": "Évaluation de la netteté",
  "core.progress.scored": "Évalué",
  "core.progress.captioning": "Rédaction des légendes",
  "core.progress.groupingPeople": "Regroupement des personnes",
  "core.progress.readingFiles": "Lecture des fichiers",
  "core.progress.indexingFiles": "Indexation",

  // --- What each on-device model is for (by model id) ---------------------
  "core.model.subject.purpose": "Détourage du sujet ou de l’arrière-plan en un clic",
  "core.model.click.purpose": "Masquage interactif en un clic (SAM 2)",
  "core.model.denoise.purpose":
    "Supprime le bruit de couleur et le bruit tacheté des hauts ISO dans Développement, en préservant les contours (NAFNet)",
  "core.model.tagger.purpose":
    "Recherchez ce que montrent les photos, dans votre propre langue, et étiquetez-les (SigLIP 2)",
  "core.model.faces.purpose":
    "Trouve les visages et signale les yeux fermés et les visages flous lors d’un tri",
  "core.model.people.purpose":
    "Reconnaît la même personne d’une photo à l’autre pour les regrouper dans Personnes",
  "core.model.erase.purpose": "Retire des personnes et des objets d’une photo dans Développement, en recréant ce qui se trouvait derrière (LaMa)",
  "core.model.subject.name": "Sélectionner le sujet",
  "core.model.click.name": "Cliquer pour sélectionner",
  "core.model.denoise.name": "Réduction du bruit par IA",
  "core.model.erase.name": "Effacer",
  "core.model.tagger.name": "Recherche et étiquetage",
  "core.model.faces.name": "Outils de visages",
  "core.model.people.name": "Personnes",

  // Failures the core classified by their root cause (src-tauri/src/errors.rs,
  // lib/coreText.ts). {file} is a file name alone, never a path.
  "core.sys.notFound": "Le fichier n’est plus là : il a peut-être été déplacé, renommé ou supprimé.",
  "core.sys.notFoundFile": "{file} n’est plus là : le fichier a peut-être été déplacé, renommé ou supprimé.",
  "core.sys.accessDenied": "SourceLight n’est pas autorisé à utiliser ce fichier ou ce dossier. Vérifiez ses autorisations.",
  "core.sys.accessDeniedFile": "SourceLight n’est pas autorisé à utiliser {file}. Vérifiez ses autorisations.",
  "core.sys.fileInUse": "Un autre programme utilise ce fichier. Fermez-le là-bas et réessayez.",
  "core.sys.fileInUseFile": "Un autre programme utilise {file}. Fermez le fichier là-bas et réessayez.",
  "core.sys.readOnly": "Le lecteur ou le fichier est en lecture seule.",
  "core.sys.readOnlyFile": "{file} se trouve sur un lecteur en lecture seule, ou est lui-même en lecture seule.",
  "core.sys.alreadyExists": "Un fichier portant ce nom existe déjà.",
  "core.sys.alreadyExistsFile": "{file} existe déjà.",
  "core.sys.fileDamaged": "Le fichier est endommagé ou incomplet.",
  "core.sys.fileDamagedFile": "{file} est endommagé ou incomplet.",
  "core.sys.rawUnsupported": "SourceLight ne sait pas encore lire ce fichier raw.",
  "core.sys.rawUnsupportedFile": "{file} est un fichier raw que SourceLight ne sait pas encore lire.",
  "core.sys.imageUnsupported": "Ce format d’image n’est pas pris en charge.",
  "core.sys.imageUnsupportedFile": "{file} est dans un format que SourceLight ne peut pas lire.",
  "core.sys.diskFull": "Le lecteur est plein. Libérez de l’espace et réessayez.",
  "core.sys.driveMissing": "Le lecteur n’est pas connecté.",
  "core.sys.outOfMemory": "L’ordinateur n’a plus de mémoire. Fermez d’autres programmes et réessayez.",
  "core.sys.catalogueBusy": "Le catalogue est occupé : une autre tâche l’utilise. Réessayez dans un instant.",
  "core.sys.catalogueDamaged": "Le catalogue est endommagé. Restaurez une sauvegarde depuis Réglages › Bibliothèque.",
  "core.sys.catalogueReadOnly": "Impossible d’écrire dans le catalogue : son dossier ou son lecteur est en lecture seule.",
  "core.sys.catalogueCannotOpen": "Le catalogue n’a pas pu être ouvert.",
  "core.sys.catalogue": "Le catalogue n’a pas pu terminer cette opération.",
  "core.sys.offline": "Connexion impossible. Vérifiez la connexion Internet et réessayez.",
  "core.sys.timedOut": "La connexion a pris trop de temps. Réessayez.",
  "core.sys.httpStatus": "Le serveur n’a pas pu répondre (erreur {status}). Réessayez plus tard.",
  "core.sys.network": "Une erreur réseau l’a interrompu. Réessayez.",
  "core.sys.aiRuntime": "Le modèle d’IA n’a pas pu s’exécuter.",
  "core.sys.raw": "Le fichier raw n’a pas pu être lu.",
  "core.sys.rawFile": "{file} n’a pas pu être lu.",
  "core.sys.image": "L’image n’a pas pu être lue ou écrite.",
  "core.sys.imageFile": "{file} n’a pas pu être lu ou écrit.",
  "core.sys.io": "Un fichier n’a pas pu être lu ou écrit.",
  "core.sys.ioFile": "{file} n’a pas pu être lu ou écrit.",
  // "{context}" is the core's own sentence for what it was doing, translated.
  "core.sys.withContext": "{context} : {cause}",
} satisfies Catalogue;
