import type { Catalogue } from "..";

export default {
  "store.resolve.done": { one: "Rechazada {count} foto {groups}.", other: "Rechazadas {count} fotos {groups}." },
  "store.resolve.groups": { one: "en {count} grupo", other: "en {count} grupos" },
  "store.resolve.skipped": { one: "Se dejó {count} ya valorado como estaba.", other: "Se dejaron {count} ya valorados como estaban." },
  "store.resolve.undo": "{key} lo deshace.",
  "store.resolve.nothing": "No hay nada que cambiar.",
  "store.resolve.allJudged": "Todos los grupos ya tienen un veredicto: no hay nada que cambiar.",
  "store.colours.noneClose": "Ninguna foto tiene colores parecidos, o aún no se han leído los colores de esta foto.",
  "store.paste.copied": "Se copiaron la ubicación y las palabras clave de {name}. Pégalas desde el menú de la foto.",
  "store.paste.place": { one: "Se aplicó el lugar de {name} a {count} foto.", other: "Se aplicó el lugar de {name} a {count} fotos." },
  "store.paste.positionToo": { one: "{count} sin posición propia tomó también su posición.", other: "{count} sin posición propia tomaron también su posición." },
  "store.paste.positionOnly": { one: "{count} foto tomó la posición de {name}.", other: "{count} fotos tomaron la posición de {name}." },
  "store.paste.keywords": { one: "Se aplicó la palabra clave de {name} a {photos}.", other: "Se aplicaron las {count} palabras clave de {name} a {photos}." },
  "store.track.title": { one: "¿Volver a situar {count} foto según su track GPX?", other: "¿Volver a situar {count} fotos según su track GPX?" },
  "store.track.message": {
    one: "Se ubicó según su hora antigua. Al volver a ubicarla, se mueve a donde estaba el track a la nueva hora. Las posiciones de la cámara o puestas a mano no cambian.",
    other: "Se ubicaron según sus horas antiguas. Al volver a ubicarlas, se mueven a donde estaba el track a las nuevas horas. Las posiciones de la cámara o puestas a mano no cambian.",
  },
  "store.track.confirm": "Volver a situar",
  "store.track.leave": "Dejarlas",
  "store.track.moved": { one: "Se movió {count} foto a lo largo de su track.", other: "Se movieron {count} fotos a lo largo de su track." },
  "store.track.outside": { one: "{count} queda ahora fuera y se quedó donde estaba.", other: "{count} quedan ahora fuera y se quedaron donde estaban." },
  "store.track.missing": { one: "El archivo de track de {count} ya no está, así que se quedó donde estaba.", other: "El archivo de track de {count} ya no está, así que se quedaron donde estaban." },
  "store.captureTime.reverted": { one: "{count} foto vuelve a la hora de su archivo.", other: "{count} fotos vuelven a la hora de su archivo." },
  "store.captureTime.nothingToRevert": "No hay nada que revertir: estas aún tienen las horas de sus archivos.",
  "store.captureTime.fromSidecars": { one: "Se tomó la hora de captura del sidecar para {count} foto.", other: "Se tomó la hora de captura del sidecar para {count} fotos." },
  "store.captureTime.changed": { one: "Se cambió la hora de captura de {count} foto.", other: "Se cambió la hora de captura de {count} fotos." },
  "store.captureTime.undated": { one: "{count} no tenía hora que mover y se dejó como estaba.", other: "{count} no tenían hora que mover y se dejaron como estaban." },
  "store.sidecarTimes.titleOne": "¿Usar la hora de captura del sidecar de {name}?",
  "store.sidecarTimes.titleMany": { one: "¿Usar la hora de captura de {count} sidecar?", other: "¿Usar las horas de captura de {count} sidecars?" },
  "store.sidecarTimes.oneDiffers": "Su sidecar dice {sidecar}; aquí es {here}.",
  "store.sidecarTimes.undated": "sin fecha",
  "store.sidecarTimes.manyDiffer": "Sus sidecars dan horas distintas de las de aquí.",
  "store.sidecarTimes.why": "Se pusieron en otra aplicación, quizá para corregir el reloj de una cámara. Los archivos no cambian, y «{revert}» en {dialog} lo deshace.",
  "store.sidecarTimes.useIts": "Usar su hora",
  "store.sidecarTimes.useTheirs": "Usar sus horas",
  "store.sidecarTimes.keep": "Mantener estas horas",
  "store.folderKeywords.added": { one: "Se añadió {count} palabra clave de los nombres de carpeta a {photos}. Está en «Carpetas», en Palabras clave.", other: "Se añadieron {count} palabras clave de los nombres de carpeta a {photos}. Están en «Carpetas», en Palabras clave." },
  "store.folderKeywords.none": "No hay nombres de carpeta que añadir. Todos son fechas, carpetas de cámara o ya están añadidos.",
  "store.folderKeywords.removed": { one: "Se quitaron las palabras clave de nombres de carpeta de {count} foto.", other: "Se quitaron las palabras clave de nombres de carpeta de {count} fotos." },
  "store.folderKeywords.noneToRemove": "No hay palabras clave de nombres de carpeta que quitar.",
  "store.keywords.confirmed": { one: "Se confirmó {count} palabra clave del etiquetador.", other: "Se confirmaron {count} palabras clave del etiquetador." },
  "store.keywords.deleted": "Se eliminó «{name}».",
  "store.keywords.deletedFrom": { one: "Se eliminó «{name}» y se quitó de {count} foto.", other: "Se eliminó «{name}» y se quitó de {count} fotos." },
  "store.types.shapePortrait": "Vertical",
  "store.types.shapeLandscape": "Horizontal",
  "store.types.shapeSquare": "Cuadrada",
  "store.types.shapePanorama": "Panorámica",
  "store.types.colourOrange": "Naranja",
  "store.types.colourTeal": "Verde azulado",
  "store.types.colourPink": "Rosa",
  "store.types.colourBrown": "Marrón",
  "store.types.colourWhite": "Blanco",
  "store.types.colourGrey": "Gris",
  "store.types.colourBlack": "Negro",
  "store.types.faces0": "Sin caras",
  "store.types.faces1": "Una cara",
  "store.types.faces2": "Dos caras",
  "store.types.faces3to5": "De tres a cinco",
  "store.types.faces6": "Seis o más",
  "store.develop.openFailed": "No se pudo abrir {name}: {error}",
  "store.develop.thisPhoto": "esta foto",
  "store.selection.narrowed": { one: "{count} foto seleccionada ya no se muestra y se ha deseleccionado.", other: "{count} fotos seleccionadas ya no se muestran y se han deseleccionado." },
  // Shared by more than one dialog
  "store.dialog.notNow": "Ahora no",

  // Offer to set up on-device AI for new photos
  "store.aiOffer.title": "¿Hacer que las fotos nuevas se puedan buscar?",
  "store.aiOffer.message": "SourceLight puede indexar y etiquetar las fotos nuevas, y encontrar a las personas que aparecen en ellas, en este equipo, para que puedas buscarlas describiéndolas. Requiere una descarga única de unos {size}. Puedes desactivarlo en Actividad.",
  "store.aiOffer.messageMetered": "SourceLight puede indexar y etiquetar las fotos nuevas, y encontrar a las personas que aparecen en ellas, en este equipo, para que puedas buscarlas describiéndolas. Requiere una descarga única de unos {size}, y parece que tienes una conexión de datos limitada. Puedes desactivarlo en Actividad.",
  "store.aiOffer.confirm": "Descargar y analizar",
  "store.aiOffer.settingUp": "Configurando la IA en el dispositivo (descarga de {size}). El progreso se muestra en Actividad.",

  // Scanning the library
  "store.scan.starting": "Iniciando",
  "store.scan.added": { one: "Se añadió {count} foto nueva.", other: "Se añadieron {count} fotos nuevas." },
  "store.scan.relinked": {
    one: "Se encontró {count} foto movida. Se conservaron sus valoraciones, palabras clave y ediciones.",
    other: "Se encontraron {count} fotos movidas. Se conservaron sus valoraciones, palabras clave y ediciones.",
  },
  "store.scan.missing": {
    one: "No se encuentra {count} foto. Puede que una unidad esté desconectada. Sus valoraciones, palabras clave y ediciones se conservan.",
    other: "No se encuentran {count} fotos. Puede que una unidad esté desconectada. Sus valoraciones, palabras clave y ediciones se conservan.",
  },
  "store.scan.failed": {
    one: "No se pudo leer {count} foto. {reason}",
    other: "No se pudieron leer {count} fotos. {reason}",
  },

  // Saving a set of photos as a collection
  "store.collection.saved": {
    one: "Se guardó {count} foto como «{name}».",
    other: "Se guardaron {count} fotos como «{name}».",
  },

  // Highlights
  "store.highlights.nothing": "No hay nada donde elegir. Todas las fotos de aquí están rechazadas, desenfocadas o tienen los ojos cerrados.",
  "store.highlights.notIndexed": "Ninguna de estas fotos está indexada aún para la búsqueda. Indéxalas primero (Actividad).",
  "store.highlights.collectionName": "Destacadas · {label}",
  "store.highlights.made": "Se guardaron las mejores {count} de {total} como «{name}».",

  // Moving rejects to the Trash / Recycle Bin
  "store.bin.title": {
    one: "¿Mover esta rechazada a {trash}?",
    other: "¿Mover {count} rechazadas a {trash}?",
  },
  "store.bin.message": "Los archivos van a {trash}, junto con sus archivos auxiliares XMP y los JPEG emparejados, y puedes restaurarlos desde allí. También se quitan del catálogo, del que antes se hace una copia de seguridad. Una rechazada con una copia que no está rechazada se conserva.",
  "store.bin.confirm": "Mover a {trash}",
  "store.bin.moved": {
    one: "Se movió {count} rechazada a {trash}: {freed} liberados.",
    other: "Se movieron {count} rechazadas a {trash}: {freed} liberados.",
  },
  "store.bin.kept": "{count} conservadas, porque una copia no está rechazada.",
  "store.bin.skipped": "{count} omitidas: falta el archivo.",
  "store.bin.nothing": "No se movió nada.",
  "store.delete.title": { one: "¿Eliminar esta foto?", other: "¿Eliminar {count} fotos?" },
  "store.delete.message": "Los archivos van a la {trash}, junto con los sidecars XMP y los JPEG emparejados, y puedes restaurarlos desde allí. También se quitan del catálogo con sus copias. Antes se hace una copia de seguridad del catálogo.",
  "store.delete.copiesTitle": { one: "¿Eliminar esta copia?", other: "¿Eliminar {count} copias?" },
  "store.delete.copiesMessage": "Solo se eliminan la copia y sus ediciones. El original y su archivo se quedan. Antes se hace una copia de seguridad del catálogo.",
  "store.delete.confirm": "Eliminar",
  "store.delete.moved": {
    one: "{count} foto movida a la {trash}: {freed} liberados.",
    other: "{count} fotos movidas a la {trash}: {freed} liberados.",
  },
  "store.delete.deleted": { one: "{count} copia eliminada.", other: "{count} copias eliminadas." },

  // Content search
  "store.search.indexTitle": "¿Indexar para la búsqueda por contenido?",
  "store.search.indexMessage": {
    one: "{count} foto aún no está indexada, así que no aparecerá en la búsqueda por contenido. Puedes seguir trabajando mientras se indexa.",
    other: "{count} fotos aún no están indexadas, así que no aparecerán en la búsqueda por contenido. Puedes seguir trabajando mientras se indexan.",
  },
  "store.search.indexConfirm": "Indexar ahora",
  "store.search.indexCancel": "Buscar de todos modos",

  // Find similar
  "store.similar.label": "Similares a {label}",
  "store.similar.none": "Ninguna foto es visualmente lo bastante parecida a esa.",

  // Duplicates, bursts and compare
  "store.duplicates.noneLooser": "No se encontraron posibles duplicados. Prueba con una similitud más baja o indexa antes la biblioteca.",
  "store.duplicates.noneNotIndexed": "No se encontraron posibles duplicados. Puede que la biblioteca aún no esté indexada.",
  "store.stacking.none": "No se encontraron ráfagas. Si la biblioteca aún no está indexada, indéxala primero.",
  "store.compare.selectTwo": "Selecciona dos o más fotos para compararlas.",

  // Undo
  "store.undo.nothing": "No hay nada que deshacer.",
  "store.undo.done": "Deshecho: {label}.",

  // Library folders
  "store.folder.addTitle": "Añadir una carpeta a la biblioteca",
  "store.folder.removeTitle": "¿Quitar {name} de la biblioteca?",
  "store.folder.removeMessage": {
    one: "Su {count} foto sale del catálogo, con sus valoraciones, palabras clave, lugares, rostros y ediciones. Los archivos del disco no se tocan y antes se hace una copia de seguridad del catálogo.",
    other: "Sus {count} fotos salen del catálogo, con sus valoraciones, palabras clave, lugares, rostros y ediciones. Los archivos del disco no se tocan y antes se hace una copia de seguridad del catálogo.",
  },
  "store.folder.removeConfirm": "Quitar carpeta",
  "store.folder.removed": {
    one: "Se quitó {name}: {count} foto olvidada.",
    other: "Se quitó {name}: {count} fotos olvidadas.",
  },
  "store.folder.locateTitle": "¿Dónde está ahora {name}?",
  "store.folder.forgetTitle": {
    one: "¿Olvidar {count} foto fuera de tus carpetas?",
    other: "¿Olvidar {count} fotos fuera de tus carpetas?",
  },
  "store.folder.forgetMessage": "Proceden de una carpeta que ya no está en la biblioteca. Al olvidarlas se quitan del catálogo, con sus valoraciones, palabras clave y ediciones (tras una copia de seguridad). Los archivos no se tocan. Para conservarlas, vuelve a añadir su carpeta.",
  "store.folder.forgetConfirm": "Olvidarlas",

  // Tagging
  "store.tagging.preparing": "Preparando",
  "store.tagging.retagTitle": "¿Volver a etiquetar todas las fotos?",
  "store.tagging.retagMessage": "Se sustituyen todas las palabras clave automáticas. Las que añadiste tú se conservan. Antes se hace una copia de seguridad del catálogo, para que puedas restaurarlo si hace falta.",
  "store.tagging.retagConfirm": "Volver a etiquetar todo",

  // Develop
  "store.develop.copySavedNamed": "Copia guardada: {name}. Ahora estás editando la copia.",
  "store.develop.copySaved": "Copia guardada. Ahora estás editando la copia.",

  // Label tables in types.ts
  "store.types.aspectFree": "Libre",
  "store.types.colourRed": "Rojo",
  "store.types.colourYellow": "Amarillo",
  "store.types.colourGreen": "Verde",
  "store.types.colourBlue": "Azul",
  "store.types.colourPurple": "Morado",
  "store.types.focalUpTo24": "Hasta 24 mm",
  "store.types.focal25to50": "25–50 mm",
  "store.types.focal51to105": "51–105 mm",
  "store.types.focal106to200": "106–200 mm",
  "store.types.focalOver200": "Más de 200 mm",
  "store.types.isoUpTo400": "ISO 400 o menos",
  "store.types.iso401to1600": "ISO 401–1600",
  "store.types.iso1601to6400": "ISO 1601–6400",
  "store.types.isoOver6400": "Más de ISO 6400",
  "store.types.apertureWider28": "ƒ/2.8 y más abierta",
  "store.types.aperture29to56": "ƒ/2.9–5.6",
  "store.types.apertureNarrower56": "Más cerrada que ƒ/5.6",
  "store.types.sortNewest": "Más recientes primero",
  "store.types.sortOldest": "Más antiguas primero",
  "store.types.sortAdded": "Añadidas recientemente",
  "store.types.sortRating": "Mejor valoradas",
  "store.types.sortName": "Nombre de archivo",
  "store.types.sortSharpest": "Más nítidas",
  // Translated 2026-10-01 (machine; native review owed).
  "store.search.stillIndexing": {
    one: "Aún se está indexando (falta {count} foto). Los resultados mejorarán cuando termine.",
    other: "Aún se está indexando (faltan {count} fotos). Los resultados mejorarán cuando termine.",
  },
  "store.copy.thePhoto": "la foto",
  "store.copy.done": "{name} copiada.",
  "store.copy.doneSized": "{name} copiada, {width} × {height}.",
  "store.copy.refused": "el portapapeles lo rechazó",
  "store.copy.failed": "No se pudo copiar {name}: {why}.",
  "store.undo.beyond": "El último cambio ({label}) no se puede deshacer. Deshacer abarca valoraciones, indicadores, etiquetas de color y ajustes de revelado pegados. Vuelve a pulsarlo para deshacer el cambio anterior.",
  "store.folder.cardTitle": "Esta carpeta está en una tarjeta de memoria",
  "store.folder.cardMessage": "Las fotos añadidas desde una tarjeta solo están en la tarjeta. Si la formateas, desaparecen junto con tus valoraciones y ediciones. Importar las copia primero a tu equipo.",
  "store.folder.cardImport": "Importar desde la tarjeta",
  "store.folder.cardNotNow": "Ahora no",
  "store.folder.cardAnywayTitle": "¿Añadir la propia tarjeta a la biblioteca?",
  "store.folder.cardAnywayMessage": "Hazlo solo si las fotos se van a quedar en esta tarjeta. SourceLight lee las carpetas de la biblioteca donde están; no las copia.",
  "store.folder.cardAnyway": "Añadir la tarjeta",
  "store.types.sortCustom": "Orden personalizado",
} satisfies Catalogue;
