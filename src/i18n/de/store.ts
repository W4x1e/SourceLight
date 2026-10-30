import type { Catalogue } from "..";

export default {
  "store.resolve.done": { one: "{count} Foto {groups} abgelehnt.", other: "{count} Fotos {groups} abgelehnt." },
  "store.resolve.groups": { one: "in {count} Gruppe", other: "in {count} Gruppen" },
  "store.resolve.skipped": { one: "{count} bereits bewertete unverändert gelassen.", other: "{count} bereits bewertete unverändert gelassen." },
  "store.resolve.undo": "{key} macht es rückgängig.",
  "store.resolve.nothing": "Nichts zu ändern.",
  "store.resolve.allJudged": "Jede Gruppe hat schon ein Urteil – nichts zu ändern.",
  "store.colours.noneClose": "Keine Fotos haben ähnliche Farben, oder die Farben dieses Fotos wurden noch nicht gelesen.",
  "store.paste.copied": "Ort und Stichwörter von {name} kopiert. Fügen Sie sie über das Fotomenü ein.",
  "store.paste.place": { one: "Ort von {name} auf {count} Foto übertragen.", other: "Ort von {name} auf {count} Fotos übertragen." },
  "store.paste.positionToo": { one: "{count} ohne eigene Position erhielt auch seine Position.", other: "{count} ohne eigene Position erhielten auch seine Position." },
  "store.paste.positionOnly": { one: "{count} Foto erhielt die Position von {name}.", other: "{count} Fotos erhielten die Position von {name}." },
  "store.paste.keywords": { one: "Stichwort von {name} auf {photos} übertragen.", other: "{count} Stichwörter von {name} auf {photos} übertragen." },
  "store.track.title": { one: "{count} Foto neu anhand seines GPX-Tracks verorten?", other: "{count} Fotos neu anhand ihres GPX-Tracks verorten?" },
  "store.track.message": {
    one: "Es wurde nach seiner alten Zeit verortet. Neu verorten setzt es dorthin, wo der Track zur neuen Zeit war. Positionen der Kamera oder von Hand gesetzte Positionen ändern sich nicht.",
    other: "Sie wurden nach ihren alten Zeiten verortet. Neu verorten setzt sie dorthin, wo der Track zu den neuen Zeiten war. Positionen der Kamera oder von Hand gesetzte Positionen ändern sich nicht.",
  },
  "store.track.confirm": "Neu verorten",
  "store.track.leave": "So lassen",
  "store.track.moved": { one: "{count} Foto entlang seines Tracks verschoben.", other: "{count} Fotos entlang ihres Tracks verschoben." },
  "store.track.outside": { one: "{count} liegt jetzt außerhalb und blieb, wo es war.", other: "{count} liegen jetzt außerhalb und blieben, wo sie waren." },
  "store.track.missing": { one: "Die Track-Datei für {count} fehlt, daher blieb es, wo es war.", other: "Die Track-Datei für {count} fehlt, daher blieben sie, wo sie waren." },
  "store.captureTime.reverted": { one: "{count} Foto auf die eigene Dateizeit zurückgesetzt.", other: "{count} Fotos auf die eigene Dateizeit zurückgesetzt." },
  "store.captureTime.nothingToRevert": "Nichts zurückzusetzen – diese haben noch ihre Dateizeiten.",
  "store.captureTime.fromSidecars": { one: "Aufnahmezeit aus dem Sidecar für {count} Foto übernommen.", other: "Aufnahmezeit aus dem Sidecar für {count} Fotos übernommen." },
  "store.captureTime.changed": { one: "Aufnahmezeit von {count} Foto geändert.", other: "Aufnahmezeit von {count} Fotos geändert." },
  "store.captureTime.undated": { one: "{count} hatte keine Zeit zum Verschieben und blieb unverändert.", other: "{count} hatten keine Zeit zum Verschieben und blieben unverändert." },
  "store.sidecarTimes.titleOne": "Aufnahmezeit aus dem Sidecar von {name} übernehmen?",
  "store.sidecarTimes.titleMany": { one: "Aufnahmezeit aus {count} Sidecar übernehmen?", other: "Aufnahmezeiten aus {count} Sidecars übernehmen?" },
  "store.sidecarTimes.oneDiffers": "Sein Sidecar nennt {sidecar}, hier steht {here}.",
  "store.sidecarTimes.undated": "ohne Datum",
  "store.sidecarTimes.manyDiffer": "Ihre Sidecars nennen andere Zeiten als die hier.",
  "store.sidecarTimes.why": "Sie wurden in einer anderen App gesetzt, vielleicht um die Uhr einer Kamera zu korrigieren. Die Dateien werden nicht verändert, und „{revert}“ in {dialog} macht es rückgängig.",
  "store.sidecarTimes.useIts": "Seine Zeit übernehmen",
  "store.sidecarTimes.useTheirs": "Ihre Zeiten übernehmen",
  "store.sidecarTimes.keep": "Diese Zeiten behalten",
  "store.folderKeywords.added": { one: "{count} Stichwort aus Ordnernamen zu {photos} hinzugefügt. Es steht unter „Ordner“ in den Stichwörtern.", other: "{count} Stichwörter aus Ordnernamen zu {photos} hinzugefügt. Sie stehen unter „Ordner“ in den Stichwörtern." },
  "store.folderKeywords.none": "Keine Ordnernamen hinzuzufügen. Es sind alles Datums- oder Kameraordner, oder sie wurden schon hinzugefügt.",
  "store.folderKeywords.removed": { one: "Stichwörter aus Ordnernamen von {count} Foto entfernt.", other: "Stichwörter aus Ordnernamen von {count} Fotos entfernt." },
  "store.folderKeywords.noneToRemove": "Keine Stichwörter aus Ordnernamen zu entfernen.",
  "store.keywords.confirmed": { one: "{count} Stichwort des Taggers bestätigt.", other: "{count} Stichwörter des Taggers bestätigt." },
  "store.keywords.deleted": "„{name}“ gelöscht.",
  "store.keywords.deletedFrom": { one: "„{name}“ gelöscht und von {count} Foto entfernt.", other: "„{name}“ gelöscht und von {count} Fotos entfernt." },
  "store.types.shapePortrait": "Hochformat",
  "store.types.shapeLandscape": "Querformat",
  "store.types.shapeSquare": "Quadratisch",
  "store.types.shapePanorama": "Panorama",
  "store.types.colourOrange": "Orange",
  "store.types.colourTeal": "Petrol",
  "store.types.colourPink": "Rosa",
  "store.types.colourBrown": "Braun",
  "store.types.colourWhite": "Weiß",
  "store.types.colourGrey": "Grau",
  "store.types.colourBlack": "Schwarz",
  "store.types.faces0": "Keine Gesichter",
  "store.types.faces1": "Ein Gesicht",
  "store.types.faces2": "Zwei Gesichter",
  "store.types.faces3to5": "Drei bis fünf",
  "store.types.faces6": "Sechs oder mehr",
  "store.develop.openFailed": "{name} ließ sich nicht öffnen: {error}",
  "store.develop.thisPhoto": "dieses Foto",
  "store.selection.narrowed": { one: "{count} ausgewähltes Foto wird nicht mehr angezeigt und wurde abgewählt.", other: "{count} ausgewählte Fotos werden nicht mehr angezeigt und wurden abgewählt." },
  // Shared by more than one dialog
  "store.dialog.notNow": "Nicht jetzt",

  // Offer to set up on-device AI for new photos
  "store.aiOffer.title": "Neue Fotos durchsuchbar machen?",
  "store.aiOffer.message": "SourceLight kann neue Fotos auf diesem Computer indexieren, taggen und Personen darauf finden, damit Sie sie durch Beschreiben suchen können. Dafür ist einmalig ein Download von etwa {size} nötig. Sie können dies unter Aktivität ausschalten.",
  "store.aiOffer.messageMetered": "SourceLight kann neue Fotos auf diesem Computer indexieren, taggen und Personen darauf finden, damit Sie sie durch Beschreiben suchen können. Dafür ist einmalig ein Download von etwa {size} nötig, und Sie scheinen eine getaktete Verbindung zu nutzen. Sie können dies unter Aktivität ausschalten.",
  "store.aiOffer.confirm": "Laden und analysieren",
  "store.aiOffer.settingUp": "KI auf dem Gerät wird eingerichtet ({size} Download). Den Fortschritt sehen Sie unter Aktivität.",

  // Scanning the library
  "store.scan.starting": "Start",
  "store.scan.added": { one: "{count} neues Foto hinzugefügt.", other: "{count} neue Fotos hinzugefügt." },
  "store.scan.relinked": {
    one: "{count} verschobenes Foto gefunden. Bewertungen, Tags und Bearbeitungen sind erhalten geblieben.",
    other: "{count} verschobene Fotos gefunden. Bewertungen, Tags und Bearbeitungen sind erhalten geblieben.",
  },
  "store.scan.missing": {
    one: "{count} Foto wurde nicht gefunden. Möglicherweise ist ein Laufwerk getrennt. Bewertungen, Tags und Bearbeitungen bleiben erhalten.",
    other: "{count} Fotos wurden nicht gefunden. Möglicherweise ist ein Laufwerk getrennt. Bewertungen, Tags und Bearbeitungen bleiben erhalten.",
  },
  "store.scan.failed": {
    one: "{count} Foto konnte nicht gelesen werden. {reason}",
    other: "{count} Fotos konnten nicht gelesen werden. {reason}",
  },

  // Saving a set of photos as a collection
  "store.collection.saved": {
    one: "{count} Foto als „{name}“ gespeichert.",
    other: "{count} Fotos als „{name}“ gespeichert.",
  },

  // Highlights
  "store.highlights.nothing": "Nichts zur Auswahl. Jedes Foto hier ist abgelehnt, unscharf oder hat geschlossene Augen.",
  "store.highlights.notIndexed": "Keines dieser Fotos ist bisher für die Suche indexiert. Indexieren Sie sie zuerst (Aktivität).",
  "store.highlights.collectionName": "Highlights · {label}",
  "store.highlights.made": "Die besten {count} von {total} als „{name}“ gespeichert.",

  // Moving rejects to the Trash / Recycle Bin. {trash} is that place's name on this system.
  "store.bin.title": {
    one: "Dieses abgelehnte Foto in den {trash} verschieben?",
    other: "{count} abgelehnte Fotos in den {trash} verschieben?",
  },
  "store.bin.message": "Die Dateien kommen in den {trash}, zusammen mit XMP-Sidecars und zugehörigen JPEGs, und lassen sich von dort wiederherstellen. Sie werden auch aus dem Katalog entfernt, der zuerst gesichert wird. Ein abgelehntes Foto mit einer nicht abgelehnten Kopie bleibt erhalten.",
  "store.bin.confirm": "In {trash} verschieben",
  "store.bin.moved": {
    one: "{count} abgelehntes Foto in den {trash} verschoben – {freed} freigegeben.",
    other: "{count} abgelehnte Fotos in den {trash} verschoben – {freed} freigegeben.",
  },
  "store.bin.kept": "{count} behalten, weil eine Kopie nicht abgelehnt ist.",
  "store.bin.skipped": "{count} übersprungen – die Datei fehlt.",
  "store.bin.nothing": "Es wurde nichts verschoben.",
  "store.delete.title": { one: "Dieses Foto löschen?", other: "{count} Fotos löschen?" },
  "store.delete.message": "Die Dateien kommen samt XMP-Sidecars und zugehörigen JPEGs in den {trash}, wo du sie wiederherstellen kannst. Sie werden auch mit allen Kopien aus dem Katalog entfernt. Der Katalog wird vorher gesichert.",
  "store.delete.copiesTitle": { one: "Diese Kopie löschen?", other: "{count} Kopien löschen?" },
  "store.delete.copiesMessage": "Nur die Kopie und ihre Bearbeitungen werden gelöscht. Das Original und seine Datei bleiben. Der Katalog wird vorher gesichert.",
  "store.delete.confirm": "Löschen",
  "store.delete.moved": {
    one: "{count} Foto in den {trash} verschoben – {freed} frei.",
    other: "{count} Fotos in den {trash} verschoben – {freed} frei.",
  },
  "store.delete.deleted": { one: "{count} Kopie gelöscht.", other: "{count} Kopien gelöscht." },

  // Content search
  "store.search.indexTitle": "Für die Inhaltssuche indexieren?",
  "store.search.indexMessage": {
    one: "{count} Foto ist noch nicht indexiert und erscheint daher nicht in der Inhaltssuche. Sie können währenddessen weiterarbeiten.",
    other: "{count} Fotos sind noch nicht indexiert und erscheinen daher nicht in der Inhaltssuche. Sie können währenddessen weiterarbeiten.",
  },
  "store.search.indexConfirm": "Jetzt indexieren",
  "store.search.indexCancel": "Trotzdem suchen",

  // Find similar
  "store.similar.label": "Ähnlich wie {label}",
  "store.similar.none": "Keine Fotos sehen diesem ähnlich genug.",

  // Duplicates, bursts and compare
  "store.duplicates.noneLooser": "Keine wahrscheinlichen Duplikate gefunden. Versuchen Sie eine lockerere Ähnlichkeit oder indexieren Sie zuerst die Bibliothek.",
  "store.duplicates.noneNotIndexed": "Keine wahrscheinlichen Duplikate gefunden. Die Bibliothek ist möglicherweise noch nicht indexiert.",
  "store.stacking.none": "Keine Serien gefunden. Falls die Bibliothek noch nicht indexiert ist, indexieren Sie sie zuerst.",
  "store.compare.selectTwo": "Wählen Sie zwei oder mehr Fotos, um sie zu vergleichen.",

  // Undo. {label} says what was undone, as the core reported it.
  "store.undo.nothing": "Nichts rückgängig zu machen.",
  "store.undo.done": "Rückgängig gemacht: {label}.",

  // Library folders
  "store.folder.addTitle": "Ordner zur Bibliothek hinzufügen",
  "store.folder.removeTitle": "{name} aus der Bibliothek entfernen?",
  "store.folder.removeMessage": {
    one: "Sein {count} Foto verlässt den Katalog, samt Bewertungen, Tags, Orten, Gesichtern und Bearbeitungen. Die Dateien auf dem Datenträger bleiben unberührt, und der Katalog wird zuvor gesichert.",
    other: "Seine {count} Fotos verlassen den Katalog, samt Bewertungen, Tags, Orten, Gesichtern und Bearbeitungen. Die Dateien auf dem Datenträger bleiben unberührt, und der Katalog wird zuvor gesichert.",
  },
  "store.folder.removeConfirm": "Ordner entfernen",
  "store.folder.removed": {
    one: "{name} entfernt – {count} Foto vergessen.",
    other: "{name} entfernt – {count} Fotos vergessen.",
  },
  "store.folder.locateTitle": "Wo ist {name} jetzt?",
  "store.folder.forgetTitle": {
    one: "{count} Foto außerhalb Ihrer Ordner vergessen?",
    other: "{count} Fotos außerhalb Ihrer Ordner vergessen?",
  },
  "store.folder.forgetMessage": "Diese stammen aus einem Ordner, der nicht mehr zur Bibliothek gehört. Beim Vergessen werden sie samt Bewertungen, Tags und Bearbeitungen aus dem Katalog entfernt (nach einem Backup). Die Dateien bleiben unberührt. Um sie zu behalten, fügen Sie ihren Ordner wieder hinzu.",
  "store.folder.forgetConfirm": "Vergessen",

  // Tagging
  "store.tagging.preparing": "Vorbereitung",
  "store.tagging.retagTitle": "Jedes Foto neu taggen?",
  "store.tagging.retagMessage": "Alle automatischen Tags werden ersetzt. Selbst hinzugefügte Tags bleiben erhalten. Der Katalog wird zuerst gesichert, damit Sie ihn bei Bedarf wiederherstellen können.",
  "store.tagging.retagConfirm": "Alles neu taggen",

  // Develop
  "store.develop.copySavedNamed": "Kopie gespeichert: {name}. Sie bearbeiten jetzt die Kopie.",
  "store.develop.copySaved": "Kopie gespeichert. Sie bearbeiten jetzt die Kopie.",

  // Label tables in types.ts (shown with t(key) where they are used)
  "store.types.aspectFree": "Frei",
  "store.types.colourRed": "Rot",
  "store.types.colourYellow": "Gelb",
  "store.types.colourGreen": "Grün",
  "store.types.colourBlue": "Blau",
  "store.types.colourPurple": "Violett",
  "store.types.focalUpTo24": "Bis 24 mm",
  "store.types.focal25to50": "25–50 mm",
  "store.types.focal51to105": "51–105 mm",
  "store.types.focal106to200": "106–200 mm",
  "store.types.focalOver200": "Über 200 mm",
  "store.types.isoUpTo400": "ISO 400 und darunter",
  "store.types.iso401to1600": "ISO 401–1600",
  "store.types.iso1601to6400": "ISO 1601–6400",
  "store.types.isoOver6400": "Über ISO 6400",
  "store.types.apertureWider28": "ƒ/2,8 und weiter offen",
  "store.types.aperture29to56": "ƒ/2,9–5,6",
  "store.types.apertureNarrower56": "Kleiner als ƒ/5,6",
  "store.types.sortNewest": "Neueste zuerst",
  "store.types.sortOldest": "Älteste zuerst",
  "store.types.sortAdded": "Zuletzt hinzugefügt",
  "store.types.sortRating": "Beste Bewertung",
  "store.types.sortName": "Dateiname",
  "store.types.sortSharpest": "Schärfste",
  // Translated 2026-10-01 (machine; native review owed).
  "store.search.stillIndexing": {
    one: "Indexierung läuft noch ({count} Foto übrig). Die Ergebnisse werden besser, sobald sie fertig ist.",
    other: "Indexierung läuft noch ({count} Fotos übrig). Die Ergebnisse werden besser, sobald sie fertig ist.",
  },
  "store.copy.thePhoto": "das Foto",
  "store.copy.done": "{name} kopiert.",
  "store.copy.doneSized": "{name} kopiert, {width} × {height}.",
  "store.copy.refused": "die Zwischenablage hat es abgelehnt",
  "store.copy.failed": "{name} konnte nicht kopiert werden: {why}.",
  "store.undo.beyond": "Die letzte Änderung ({label}) kann nicht rückgängig gemacht werden. Rückgängig umfasst Bewertungen, Markierungen, Farbmarkierungen und eingefügte Entwicklungseinstellungen. Drücken Sie erneut, um die Änderung davor rückgängig zu machen.",
  "store.folder.cardTitle": "Dieser Ordner liegt auf einer Speicherkarte",
  "store.folder.cardMessage": "Von einer Karte hinzugefügte Fotos bleiben nur auf der Karte. Wenn Sie sie formatieren, sind die Fotos weg, samt Bewertungen und Bearbeitungen. Der Import kopiert sie zuerst auf Ihren Computer.",
  "store.folder.cardImport": "Von der Karte importieren",
  "store.folder.cardNotNow": "Nicht jetzt",
  "store.folder.cardAnywayTitle": "Die Karte selbst zur Bibliothek hinzufügen?",
  "store.folder.cardAnywayMessage": "Tun Sie das nur, wenn die Fotos auf dieser Karte bleiben. SourceLight liest Bibliotheksordner dort, wo sie sind, und kopiert sie nicht.",
  "store.folder.cardAnyway": "Karte hinzufügen",
  "store.types.sortCustom": "Eigene Reihenfolge",
} satisfies Catalogue;
