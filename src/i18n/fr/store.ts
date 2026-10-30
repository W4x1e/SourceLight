import type { Catalogue } from "..";

export default {
  "store.resolve.done": { one: "{count} photo rejetée {groups}.", other: "{count} photos rejetées {groups}." },
  "store.resolve.groups": { one: "dans {count} groupe", other: "dans {count} groupes" },
  "store.resolve.skipped": { one: "{count} déjà jugé laissé tel quel.", other: "{count} déjà jugés laissés tels quels." },
  "store.resolve.undo": "{key} l’annule.",
  "store.resolve.nothing": "Rien à changer.",
  "store.resolve.allJudged": "Chaque groupe a déjà un verdict — rien à changer.",
  "store.colours.noneClose": "Aucune photo n’a de couleurs similaires, ou les couleurs de cette photo n’ont pas encore été lues.",
  "store.paste.copied": "Lieu et mots-clés de {name} copiés. Collez-les depuis le menu de la photo.",
  "store.paste.place": { one: "Lieu de {name} appliqué à {count} photo.", other: "Lieu de {name} appliqué à {count} photos." },
  "store.paste.positionToo": { one: "{count} sans position propre a aussi pris sa position.", other: "{count} sans position propre ont aussi pris sa position." },
  "store.paste.positionOnly": { one: "{count} photo a reçu la position de {name}.", other: "{count} photos ont reçu la position de {name}." },
  "store.paste.keywords": { one: "Mot-clé de {name} appliqué à {photos}.", other: "{count} mots-clés de {name} appliqués à {photos}." },
  "store.track.title": { one: "Replacer {count} photo selon sa trace GPX ?", other: "Replacer {count} photos selon leur trace GPX ?" },
  "store.track.message": {
    one: "Elle a été placée selon son ancienne heure. La replacer la déplace là où était la trace à la nouvelle heure. Les positions de l’appareil ou fixées à la main ne changent pas.",
    other: "Elles ont été placées selon leurs anciennes heures. Les replacer les déplace là où était la trace aux nouvelles heures. Les positions de l’appareil ou fixées à la main ne changent pas.",
  },
  "store.track.confirm": "Replacer",
  "store.track.leave": "Les laisser",
  "store.track.moved": { one: "{count} photo déplacée le long de sa trace.", other: "{count} photos déplacées le long de leur trace." },
  "store.track.outside": { one: "{count} tombe désormais en dehors et reste où elle était.", other: "{count} tombent désormais en dehors et restent où elles étaient." },
  "store.track.missing": { one: "Le fichier de trace de {count} a disparu, elle reste donc en place.", other: "Le fichier de trace de {count} a disparu, elles restent donc en place." },
  "store.captureTime.reverted": { one: "{count} photo remise à l’heure de son fichier.", other: "{count} photos remises à l’heure de leur fichier." },
  "store.captureTime.nothingToRevert": "Rien à rétablir — elles ont encore les heures de leurs fichiers.",
  "store.captureTime.fromSidecars": { one: "Heure de prise de vue du sidecar reprise pour {count} photo.", other: "Heure de prise de vue du sidecar reprise pour {count} photos." },
  "store.captureTime.changed": { one: "Heure de prise de vue modifiée pour {count} photo.", other: "Heure de prise de vue modifiée pour {count} photos." },
  "store.captureTime.undated": { one: "{count} n’avait pas d’heure à décaler et a été laissée telle quelle.", other: "{count} n’avaient pas d’heure à décaler et ont été laissées telles quelles." },
  "store.sidecarTimes.titleOne": "Utiliser l’heure de prise de vue du sidecar de {name} ?",
  "store.sidecarTimes.titleMany": { one: "Utiliser l’heure de prise de vue de {count} sidecar ?", other: "Utiliser les heures de prise de vue de {count} sidecars ?" },
  "store.sidecarTimes.oneDiffers": "Son sidecar indique {sidecar} ; ici, c’est {here}.",
  "store.sidecarTimes.undated": "sans date",
  "store.sidecarTimes.manyDiffer": "Leurs sidecars donnent d’autres heures que celles d’ici.",
  "store.sidecarTimes.why": "Elles ont été définies dans une autre application, peut-être pour corriger l’horloge d’un appareil. Les fichiers ne sont pas modifiés, et « {revert} » dans {dialog} l’annule.",
  "store.sidecarTimes.useIts": "Utiliser son heure",
  "store.sidecarTimes.useTheirs": "Utiliser leurs heures",
  "store.sidecarTimes.keep": "Garder ces heures",
  "store.folderKeywords.added": { one: "{count} mot-clé tiré des noms de dossier ajouté à {photos}. Il est sous « Dossiers » dans Mots-clés.", other: "{count} mots-clés tirés des noms de dossier ajoutés à {photos}. Ils sont sous « Dossiers » dans Mots-clés." },
  "store.folderKeywords.none": "Aucun nom de dossier à ajouter. Ce sont tous des dates, des dossiers d’appareil, ou ils sont déjà ajoutés.",
  "store.folderKeywords.removed": { one: "Mots-clés tirés des noms de dossier retirés de {count} photo.", other: "Mots-clés tirés des noms de dossier retirés de {count} photos." },
  "store.folderKeywords.noneToRemove": "Aucun mot-clé tiré des noms de dossier à retirer.",
  "store.keywords.confirmed": { one: "{count} mot-clé de l’étiqueteur confirmé.", other: "{count} mots-clés de l’étiqueteur confirmés." },
  "store.keywords.deleted": "« {name} » supprimé.",
  "store.keywords.deletedFrom": { one: "« {name} » supprimé et retiré de {count} photo.", other: "« {name} » supprimé et retiré de {count} photos." },
  "store.types.shapePortrait": "Portrait",
  "store.types.shapeLandscape": "Paysage",
  "store.types.shapeSquare": "Carré",
  "store.types.shapePanorama": "Panorama",
  "store.types.colourOrange": "Orange",
  "store.types.colourTeal": "Bleu canard",
  "store.types.colourPink": "Rose",
  "store.types.colourBrown": "Marron",
  "store.types.colourWhite": "Blanc",
  "store.types.colourGrey": "Gris",
  "store.types.colourBlack": "Noir",
  "store.types.faces0": "Aucun visage",
  "store.types.faces1": "Un visage",
  "store.types.faces2": "Deux visages",
  "store.types.faces3to5": "Trois à cinq",
  "store.types.faces6": "Six ou plus",
  "store.develop.openFailed": "Impossible d’ouvrir {name} : {error}",
  "store.develop.thisPhoto": "cette photo",
  "store.selection.narrowed": { one: "{count} photo sélectionnée n’est plus affichée et a été désélectionnée.", other: "{count} photos sélectionnées ne sont plus affichées et ont été désélectionnées." },
  // Shared by more than one dialog
  "store.dialog.notNow": "Plus tard",

  // Offer to set up on-device AI for new photos
  "store.aiOffer.title": "Rendre les nouvelles photos consultables par la recherche ?",
  "store.aiOffer.message": "SourceLight peut indexer et étiqueter les nouvelles photos et y trouver les personnes, sur cet ordinateur, pour que vous puissiez les rechercher en les décrivant. Cela nécessite un téléchargement unique d’environ {size}. Vous pouvez le désactiver dans Activité.",
  "store.aiOffer.messageMetered": "SourceLight peut indexer et étiqueter les nouvelles photos et y trouver les personnes, sur cet ordinateur, pour que vous puissiez les rechercher en les décrivant. Cela nécessite un téléchargement unique d’environ {size}, et vous semblez être sur une connexion limitée. Vous pouvez le désactiver dans Activité.",
  "store.aiOffer.confirm": "Télécharger et analyser",
  "store.aiOffer.settingUp": "Configuration de l’IA sur l’appareil (téléchargement de {size}). La progression s’affiche dans Activité.",

  // Scanning the library
  "store.scan.starting": "Démarrage",
  "store.scan.added": { one: "{count} nouvelle photo ajoutée.", other: "{count} nouvelles photos ajoutées." },
  "store.scan.relinked": {
    one: "{count} photo déplacée retrouvée. Ses notes, mots-clés et retouches ont été conservés.",
    other: "{count} photos déplacées retrouvées. Leurs notes, mots-clés et retouches ont été conservés.",
  },
  "store.scan.missing": {
    one: "{count} photo est introuvable. Un disque est peut-être déconnecté. Ses notes, mots-clés et retouches sont conservés.",
    other: "{count} photos sont introuvables. Un disque est peut-être déconnecté. Leurs notes, mots-clés et retouches sont conservés.",
  },
  "store.scan.failed": {
    one: "{count} photo n’a pas pu être lue. {reason}",
    other: "{count} photos n’ont pas pu être lues. {reason}",
  },

  // Saving a set of photos as a collection
  "store.collection.saved": {
    one: "{count} photo enregistrée sous « {name} ».",
    other: "{count} photos enregistrées sous « {name} ».",
  },

  // Highlights
  "store.highlights.nothing": "Rien à choisir. Toutes les photos ici sont rejetées, floues ou ont les yeux fermés.",
  "store.highlights.notIndexed": "Aucune de ces photos n’est encore indexée pour la recherche. Indexez-les d’abord (Activité).",
  "store.highlights.collectionName": "Temps forts · {label}",
  "store.highlights.made": "Les {count} meilleures sur {total} ont été enregistrées sous « {name} ».",

  // Moving rejects to the Trash / Recycle Bin
  "store.bin.title": {
    one: "Déplacer cette photo rejetée vers la {trash} ?",
    other: "Déplacer {count} photos rejetées vers la {trash} ?",
  },
  "store.bin.message": "Les fichiers vont dans la {trash}, avec leurs fichiers annexes XMP et les JPEG associés, et vous pouvez les y restaurer. Ils sont aussi retirés du catalogue, qui est d’abord sauvegardé. Une photo rejetée dont une copie n’est pas rejetée est conservée.",
  "store.bin.confirm": "Déplacer vers la {trash}",
  "store.bin.moved": {
    one: "{count} photo rejetée déplacée vers la {trash} — {freed} libérés.",
    other: "{count} photos rejetées déplacées vers la {trash} — {freed} libérés.",
  },
  "store.bin.kept": "{count} conservées, car une copie n’est pas rejetée.",
  "store.bin.skipped": "{count} ignorées — le fichier est introuvable.",
  "store.bin.nothing": "Rien n’a été déplacé.",
  "store.delete.title": { one: "Supprimer cette photo ?", other: "Supprimer {count} photos ?" },
  "store.delete.message": "Les fichiers vont dans la {trash}, avec leurs sidecars XMP et JPEG associés, et vous pouvez les y restaurer. Ils sont aussi retirés du catalogue avec leurs copies. Le catalogue est sauvegardé avant.",
  "store.delete.copiesTitle": { one: "Supprimer cette copie ?", other: "Supprimer {count} copies ?" },
  "store.delete.copiesMessage": "Seules la copie et ses retouches disparaissent. L’original et son fichier restent. Le catalogue est sauvegardé avant.",
  "store.delete.confirm": "Supprimer",
  "store.delete.moved": {
    one: "{count} photo déplacée vers la {trash} — {freed} libérés.",
    other: "{count} photos déplacées vers la {trash} — {freed} libérés.",
  },
  "store.delete.deleted": { one: "{count} copie supprimée.", other: "{count} copies supprimées." },

  // Content search
  "store.search.indexTitle": "Indexer pour la recherche par contenu ?",
  "store.search.indexMessage": {
    one: "{count} photo n’est pas encore indexée et n’apparaîtra donc pas dans la recherche par contenu. Vous pouvez continuer à travailler pendant l’indexation.",
    other: "{count} photos ne sont pas encore indexées et n’apparaîtront donc pas dans la recherche par contenu. Vous pouvez continuer à travailler pendant l’indexation.",
  },
  "store.search.indexConfirm": "Indexer maintenant",
  "store.search.indexCancel": "Rechercher quand même",

  // Find similar
  "store.similar.label": "Similaires à {label}",
  "store.similar.none": "Aucune photo n’est visuellement assez proche de celle-ci.",

  // Duplicates, bursts and compare
  "store.duplicates.noneLooser": "Aucun doublon probable trouvé. Essayez une similarité plus large, ou indexez d’abord la bibliothèque.",
  "store.duplicates.noneNotIndexed": "Aucun doublon probable trouvé. La bibliothèque n’est peut-être pas encore indexée.",
  "store.stacking.none": "Aucune rafale trouvée. Si la bibliothèque n’est pas encore indexée, indexez-la d’abord.",
  "store.compare.selectTwo": "Sélectionnez au moins deux photos pour les comparer.",

  // Undo
  "store.undo.nothing": "Rien à annuler.",
  "store.undo.done": "Annulé : {label}.",

  // Library folders
  "store.folder.addTitle": "Ajouter un dossier à la bibliothèque",
  "store.folder.removeTitle": "Retirer {name} de la bibliothèque ?",
  "store.folder.removeMessage": {
    one: "Sa {count} photo quitte le catalogue, avec ses notes, mots-clés, lieux, visages et retouches. Les fichiers sur le disque ne sont pas touchés, et le catalogue est d’abord sauvegardé.",
    other: "Ses {count} photos quittent le catalogue, avec leurs notes, mots-clés, lieux, visages et retouches. Les fichiers sur le disque ne sont pas touchés, et le catalogue est d’abord sauvegardé.",
  },
  "store.folder.removeConfirm": "Retirer le dossier",
  "store.folder.removed": {
    one: "{name} retiré — {count} photo oubliée.",
    other: "{name} retiré — {count} photos oubliées.",
  },
  "store.folder.locateTitle": "Où se trouve {name} maintenant ?",
  "store.folder.forgetTitle": {
    one: "Oublier {count} photo située hors de vos dossiers ?",
    other: "Oublier {count} photos situées hors de vos dossiers ?",
  },
  "store.folder.forgetMessage": "Elles proviennent d’un dossier qui ne fait plus partie de la bibliothèque. Les oublier les retire du catalogue, avec leurs notes, mots-clés et retouches (après une sauvegarde). Les fichiers ne sont pas touchés. Pour les garder, ajoutez à nouveau leur dossier.",
  "store.folder.forgetConfirm": "Les oublier",

  // Tagging
  "store.tagging.preparing": "Préparation",
  "store.tagging.retagTitle": "Refaire les mots-clés de toutes les photos ?",
  "store.tagging.retagMessage": "Tous les mots-clés automatiques sont remplacés. Ceux que vous avez ajoutés vous-même sont conservés. Le catalogue est d’abord sauvegardé, pour que vous puissiez le restaurer si besoin.",
  "store.tagging.retagConfirm": "Tout refaire",

  // Develop
  "store.develop.copySavedNamed": "Copie enregistrée : {name}. Vous retouchez maintenant la copie.",
  "store.develop.copySaved": "Copie enregistrée. Vous retouchez maintenant la copie.",

  // Label tables in types.ts
  "store.types.aspectFree": "Libre",
  "store.types.colourRed": "Rouge",
  "store.types.colourYellow": "Jaune",
  "store.types.colourGreen": "Vert",
  "store.types.colourBlue": "Bleu",
  "store.types.colourPurple": "Violet",
  "store.types.focalUpTo24": "Jusqu’à 24 mm",
  "store.types.focal25to50": "25–50 mm",
  "store.types.focal51to105": "51–105 mm",
  "store.types.focal106to200": "106–200 mm",
  "store.types.focalOver200": "Plus de 200 mm",
  "store.types.isoUpTo400": "ISO 400 et moins",
  "store.types.iso401to1600": "ISO 401–1600",
  "store.types.iso1601to6400": "ISO 1601–6400",
  "store.types.isoOver6400": "Plus de ISO 6400",
  "store.types.apertureWider28": "ƒ/2.8 et plus ouvert",
  "store.types.aperture29to56": "ƒ/2.9–5.6",
  "store.types.apertureNarrower56": "Plus fermé que ƒ/5.6",
  "store.types.sortNewest": "Plus récentes d’abord",
  "store.types.sortOldest": "Plus anciennes d’abord",
  "store.types.sortAdded": "Ajoutées récemment",
  "store.types.sortRating": "Mieux notées",
  "store.types.sortName": "Nom de fichier",
  "store.types.sortSharpest": "Plus nettes",
  // Translated 2026-10-01 (machine; native review owed).
  "store.search.stillIndexing": {
    one: "Indexation en cours ({count} photo restante). Les résultats s’amélioreront une fois terminée.",
    other: "Indexation en cours ({count} photos restantes). Les résultats s’amélioreront une fois terminée.",
  },
  "store.copy.thePhoto": "la photo",
  "store.copy.done": "{name} copiée.",
  "store.copy.doneSized": "{name} copiée, {width} × {height}.",
  "store.copy.refused": "le presse-papiers l’a refusée",
  "store.copy.failed": "Impossible de copier {name} : {why}.",
  "store.undo.beyond": "La dernière modification ({label}) ne peut pas être annulée. Annuler couvre les notes, drapeaux, étiquettes de couleur et réglages de développement collés. Appuyez à nouveau pour annuler la modification précédente.",
  "store.folder.cardTitle": "Ce dossier est sur une carte mémoire",
  "store.folder.cardMessage": "Les photos ajoutées depuis une carte ne sont que sur la carte. Si vous la formatez, elles disparaissent, avec vos notes et retouches. L’importation les copie d’abord sur votre ordinateur.",
  "store.folder.cardImport": "Importer depuis la carte",
  "store.folder.cardNotNow": "Pas maintenant",
  "store.folder.cardAnywayTitle": "Ajouter la carte elle-même à la bibliothèque ?",
  "store.folder.cardAnywayMessage": "Ne le faites que si les photos resteront sur cette carte. SourceLight lit les dossiers de la bibliothèque là où ils sont ; il ne les copie pas.",
  "store.folder.cardAnyway": "Ajouter la carte",
  "store.types.sortCustom": "Ordre personnalisé",
} satisfies Catalogue;
