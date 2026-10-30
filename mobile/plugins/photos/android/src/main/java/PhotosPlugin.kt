package app.sourcelight.photos

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.util.Size
import androidx.core.content.ContextCompat
import androidx.exifinterface.media.ExifInterface
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.Executors

@InvokeArg
class PhotosArgs {
    /** An album's bucket id, or null for every photo. */
    var album: String? = null
}

@InvokeArg
class OriginalArgs {
    var id: Long = 0
    /** Longest side to edit at: anything larger is decoded down to it. */
    var maxEdge: Int = 4096
    /** Where to write the file. */
    var path: String = ""
}

@InvokeArg
class FileArgs {
    var id: Long = 0
    /** Where to write it. */
    var path: String = ""
}

@InvokeArg
class MetadataArgs {
    /** The photo whose details to copy. */
    var id: Long = 0
    /** The JPEG to copy them into. */
    var path: String = ""
    /** When it was taken, for a photo whose file doesn't say. */
    var takenMs: Long = 0
}

@InvokeArg
class ImageArgs {
    var id: Long = 0
    /** Longest side, in pixels. */
    var size: Int = 384
    /** Where to write the JPEG. */
    var path: String = ""
}

/**
 * The phone's own photos for On this phone (docs/MOBILE.md section 4): whether
 * the app may see them, the albums, the photos newest first, and JPEGs of any
 * of them at a given size. Android's own MediaStore and decoders do the work,
 * so HEIC and every format the phone reads come for free.
 */
@TauriPlugin(
    permissions = [
        Permission(strings = [Manifest.permission.READ_MEDIA_IMAGES], alias = "images"),
        Permission(strings = [Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED], alias = "selected"),
        Permission(strings = [Manifest.permission.READ_EXTERNAL_STORAGE], alias = "storage"),
        Permission(strings = [Manifest.permission.ACCESS_MEDIA_LOCATION], alias = "location"),
    ]
)
class PhotosPlugin(private val activity: Activity) : Plugin(activity) {
    companion object {
        /** What a saved copy keeps of the original's EXIF. */
        private val COPIED = arrayOf(
            ExifInterface.TAG_DATETIME_ORIGINAL,
            ExifInterface.TAG_OFFSET_TIME_ORIGINAL,
            ExifInterface.TAG_SUBSEC_TIME_ORIGINAL,
            ExifInterface.TAG_DATETIME_DIGITIZED,
            ExifInterface.TAG_MAKE,
            ExifInterface.TAG_MODEL,
            ExifInterface.TAG_LENS_MAKE,
            ExifInterface.TAG_LENS_MODEL,
            ExifInterface.TAG_F_NUMBER,
            ExifInterface.TAG_EXPOSURE_TIME,
            ExifInterface.TAG_PHOTOGRAPHIC_SENSITIVITY,
            ExifInterface.TAG_FOCAL_LENGTH,
            ExifInterface.TAG_FOCAL_LENGTH_IN_35MM_FILM,
            ExifInterface.TAG_FLASH,
            ExifInterface.TAG_GPS_LATITUDE,
            ExifInterface.TAG_GPS_LATITUDE_REF,
            ExifInterface.TAG_GPS_LONGITUDE,
            ExifInterface.TAG_GPS_LONGITUDE_REF,
            ExifInterface.TAG_GPS_ALTITUDE,
            ExifInterface.TAG_GPS_ALTITUDE_REF,
            ExifInterface.TAG_GPS_DATESTAMP,
            ExifInterface.TAG_GPS_TIMESTAMP,
        )
    }

    /** Off the main thread; a few at once, as a grid asks for many thumbnails. */
    private val worker = Executors.newFixedThreadPool(3)

    private fun has(permission: String) =
        ContextCompat.checkSelfPermission(activity, permission) == PackageManager.PERMISSION_GRANTED

    /** "all", "some" (Android 14's chosen photos only) or "none". */
    private fun currentAccess(): String = when {
        Build.VERSION.SDK_INT >= 33 && has(Manifest.permission.READ_MEDIA_IMAGES) -> "all"
        Build.VERSION.SDK_INT >= 34 && has(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED) -> "some"
        Build.VERSION.SDK_INT < 33 && has(Manifest.permission.READ_EXTERNAL_STORAGE) -> "all"
        else -> "none"
    }

    @Command
    fun access(invoke: Invoke) {
        invoke.resolve(JSObject().put("access", currentAccess()))
    }

    /** Asks, or with chosen photos only, lets the person choose again. */
    @Command
    fun requestAccess(invoke: Invoke) {
        val aliases = when {
            Build.VERSION.SDK_INT >= 34 -> arrayOf("images", "selected", "location")
            Build.VERSION.SDK_INT >= 33 -> arrayOf("images", "location")
            Build.VERSION.SDK_INT >= 29 -> arrayOf("storage", "location")
            else -> arrayOf("storage")
        }
        requestPermissionForAliases(aliases, invoke, "accessAnswered")
    }

    @PermissionCallback
    fun accessAnswered(invoke: Invoke) {
        invoke.resolve(JSObject().put("access", currentAccess()))
    }

    /** Albums (MediaStore buckets) with their photo counts, largest first. */
    @Command
    fun albums(invoke: Invoke) {
        worker.execute {
            try {
                val found = LinkedHashMap<String, Pair<String, Int>>()
                activity.contentResolver.query(
                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                    arrayOf(MediaStore.Images.Media.BUCKET_ID, MediaStore.Images.Media.BUCKET_DISPLAY_NAME),
                    null,
                    null,
                    null
                )?.use { c ->
                    val idCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.BUCKET_ID)
                    val nameCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.BUCKET_DISPLAY_NAME)
                    while (c.moveToNext()) {
                        val id = c.getString(idCol) ?: continue
                        val name = c.getString(nameCol) ?: ""
                        val seen = found[id]
                        found[id] = Pair(name, (seen?.second ?: 0) + 1)
                    }
                }
                val out = JSArray()
                for ((id, album) in found.entries.sortedByDescending { it.value.second }) {
                    out.put(JSObject().put("id", id).put("name", album.first).put("count", album.second))
                }
                invoke.resolve(JSObject().put("albums", out))
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /**
     * The photos, newest first, each `[id, takenMs, modified, width, height,
     * raw]`: compact, as a phone can hold tens of thousands. `modified`
     * changes when the photo does, so a cached thumbnail knows it is stale.
     */
    @Command
    fun photos(invoke: Invoke) {
        val args = invoke.parseArgs(PhotosArgs::class.java)
        worker.execute {
            try {
                // Sorted here rather than by MediaStore: a photo with no
                // capture time goes by when it arrived, not to the end.
                val rows = ArrayList<LongArray>()
                val selection = args.album?.let { "${MediaStore.Images.Media.BUCKET_ID} = ?" }
                val selectionArgs = args.album?.let { arrayOf(it) }
                activity.contentResolver.query(
                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                    arrayOf(
                        MediaStore.Images.Media._ID,
                        MediaStore.Images.Media.DATE_TAKEN,
                        MediaStore.Images.Media.DATE_ADDED,
                        MediaStore.Images.Media.DATE_MODIFIED,
                        MediaStore.Images.Media.WIDTH,
                        MediaStore.Images.Media.HEIGHT,
                        MediaStore.Images.Media.ORIENTATION,
                        MediaStore.Images.Media.MIME_TYPE,
                    ),
                    selection,
                    selectionArgs,
                    null
                )?.use { c ->
                    val id = c.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
                    val taken = c.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_TAKEN)
                    val added = c.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_ADDED)
                    val modified = c.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_MODIFIED)
                    val width = c.getColumnIndexOrThrow(MediaStore.Images.Media.WIDTH)
                    val height = c.getColumnIndexOrThrow(MediaStore.Images.Media.HEIGHT)
                    val orientation = c.getColumnIndexOrThrow(MediaStore.Images.Media.ORIENTATION)
                    val mime = c.getColumnIndexOrThrow(MediaStore.Images.Media.MIME_TYPE)
                    while (c.moveToNext()) {
                        // A photo with no capture time sorts by when it arrived.
                        val takenMs = if (c.isNull(taken)) c.getLong(added) * 1000 else c.getLong(taken)
                        val turned = c.getInt(orientation) % 180 != 0
                        val (w, h) = Pair(c.getInt(width), c.getInt(height))
                        val type = c.getString(mime) ?: ""
                        val raw = type.contains("dng") || type.contains("raw")
                        rows.add(
                            longArrayOf(
                                c.getLong(id),
                                takenMs,
                                c.getLong(modified),
                                (if (turned) h else w).toLong(),
                                (if (turned) w else h).toLong(),
                                if (raw) 1L else 0L,
                            )
                        )
                    }
                }
                rows.sortWith(compareByDescending<LongArray> { it[1] }.thenByDescending { it[0] })
                val out = JSArray()
                for (row in rows) {
                    val item = JSArray()
                    for (v in row) item.put(v)
                    out.put(item)
                }
                invoke.resolve(JSObject().put("photos", out))
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /** A JPEG of photo `id`, upright, no larger than `size`, written to `path`. */
    @Command
    fun image(invoke: Invoke) {
        val args = invoke.parseArgs(ImageArgs::class.java)
        worker.execute {
            try {
                val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, args.id)
                val bitmap = decode(uri, args.size)
                val target = File(args.path)
                target.parentFile?.mkdirs()
                // Written aside and moved in, so a reader never sees half a file.
                val partial = File(target.path + ".part")
                FileOutputStream(partial).use { bitmap.compress(Bitmap.CompressFormat.JPEG, 85, it) }
                bitmap.recycle()
                if (!partial.renameTo(target)) {
                    partial.delete()
                    throw java.io.IOException("Couldn't save the image.")
                }
                invoke.resolve(JSObject())
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /**
     * The photo to edit, written to `path`: its own bytes when it is a JPEG,
     * PNG or WebP no larger than `maxEdge`, so nothing is lost; otherwise
     * (HEIC, a DNG, a 50 MP photo) decoded by Android, upright, down to
     * `maxEdge` and saved as a high-quality JPEG. Resolves the photo's name
     * and capture time, and whether its bytes were copied as they are.
     */
    @Command
    fun original(invoke: Invoke) {
        val args = invoke.parseArgs(OriginalArgs::class.java)
        worker.execute {
            try {
                val resolver = activity.contentResolver
                val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, args.id)
                var name = "photo.jpg"
                var mime = ""
                var takenMs = 0L
                var longest = 0
                resolver.query(
                    uri,
                    arrayOf(
                        MediaStore.Images.Media.DISPLAY_NAME,
                        MediaStore.Images.Media.MIME_TYPE,
                        MediaStore.Images.Media.DATE_TAKEN,
                        MediaStore.Images.Media.DATE_ADDED,
                        MediaStore.Images.Media.WIDTH,
                        MediaStore.Images.Media.HEIGHT,
                    ),
                    null,
                    null,
                    null
                )?.use { c ->
                    if (c.moveToFirst()) {
                        name = c.getString(0) ?: name
                        mime = c.getString(1) ?: ""
                        takenMs = if (c.isNull(2)) c.getLong(3) * 1000 else c.getLong(2)
                        longest = maxOf(c.getInt(4), c.getInt(5))
                    }
                }
                val target = File(args.path)
                target.parentFile?.mkdirs()
                val partial = File(target.path + ".part")
                val plain = mime == "image/jpeg" || mime == "image/png" || mime == "image/webp"
                val copied = plain && longest in 1..args.maxEdge
                if (copied) {
                    val input = resolver.openInputStream(original(uri))
                        ?: throw java.io.IOException("Couldn't read this photo.")
                    input.use { i -> FileOutputStream(partial).use { i.copyTo(it) } }
                } else {
                    val bitmap = decode(uri, args.maxEdge)
                    FileOutputStream(partial).use { bitmap.compress(Bitmap.CompressFormat.JPEG, 95, it) }
                    bitmap.recycle()
                }
                if (!partial.renameTo(target)) {
                    partial.delete()
                    throw java.io.IOException("Couldn't save the photo to edit.")
                }
                invoke.resolve(JSObject().put("name", name).put("takenMs", takenMs).put("copied", copied))
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /**
     * Photo `id`'s file exactly as stored, location included when Android
     * allows it, written to `path`: what Add to library sends. Resolves its
     * name, capture time and type.
     */
    @Command
    fun file(invoke: Invoke) {
        val args = invoke.parseArgs(FileArgs::class.java)
        worker.execute {
            try {
                val resolver = activity.contentResolver
                val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, args.id)
                var name: String? = null
                var mime = ""
                var takenMs = 0L
                resolver.query(
                    uri,
                    arrayOf(
                        MediaStore.Images.Media.DISPLAY_NAME,
                        MediaStore.Images.Media.MIME_TYPE,
                        MediaStore.Images.Media.DATE_TAKEN,
                        MediaStore.Images.Media.DATE_ADDED,
                    ),
                    null,
                    null,
                    null
                )?.use { c ->
                    if (c.moveToFirst()) {
                        name = c.getString(0)
                        mime = c.getString(1) ?: ""
                        takenMs = if (c.isNull(2)) c.getLong(3) * 1000 else c.getLong(2)
                    }
                }
                if (name == null) throw java.io.IOException("This photo is no longer on the phone.")
                val target = File(args.path)
                target.parentFile?.mkdirs()
                val partial = File(target.path + ".part")
                val input = resolver.openInputStream(original(uri))
                    ?: throw java.io.IOException("Couldn't read this photo.")
                input.use { i -> FileOutputStream(partial).use { i.copyTo(it) } }
                if (!partial.renameTo(target)) {
                    partial.delete()
                    throw java.io.IOException("Couldn't copy this photo.")
                }
                invoke.resolve(JSObject().put("name", name).put("takenMs", takenMs).put("mime", mime))
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /**
     * Photo `id`'s camera details, capture time and location, written into the
     * JPEG at `path`: a saved copy keeps what the original says about itself.
     * Upright, as the copy is. A photo whose file names no capture time gets
     * `takenMs`, so the gallery sorts the copy beside it.
     */
    @Command
    fun copyMetadata(invoke: Invoke) {
        val args = invoke.parseArgs(MetadataArgs::class.java)
        worker.execute {
            try {
                val uri = original(ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, args.id))
                val from = activity.contentResolver.openInputStream(uri)?.use { ExifInterface(it) }
                val to = ExifInterface(args.path)
                if (from != null) {
                    for (tag in COPIED) from.getAttribute(tag)?.let { to.setAttribute(tag, it) }
                }
                if (to.getAttribute(ExifInterface.TAG_DATETIME_ORIGINAL) == null && args.takenMs > 0) {
                    // Local time with its offset: without one, the gallery
                    // reads the time as UTC and sorts the copy hours away.
                    val zone = java.util.TimeZone.getDefault()
                    val stamp = java.text.SimpleDateFormat("yyyy:MM:dd HH:mm:ss", java.util.Locale.US)
                        .apply { timeZone = zone }
                        .format(java.util.Date(args.takenMs))
                    val minutes = zone.getOffset(args.takenMs) / 60_000
                    val offset = String.format(
                        java.util.Locale.US,
                        "%s%02d:%02d",
                        if (minutes < 0) "-" else "+",
                        Math.abs(minutes) / 60,
                        Math.abs(minutes) % 60
                    )
                    to.setAttribute(ExifInterface.TAG_DATETIME_ORIGINAL, stamp)
                    to.setAttribute(ExifInterface.TAG_OFFSET_TIME_ORIGINAL, offset)
                }
                to.setAttribute(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL.toString())
                to.setAttribute(ExifInterface.TAG_SOFTWARE, "SourceLight")
                to.saveAttributes()
                invoke.resolve(JSObject())
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /** The photo as stored, location included, when Android allows it. */
    private fun original(uri: Uri): Uri =
        if (Build.VERSION.SDK_INT >= 29 && has(Manifest.permission.ACCESS_MEDIA_LOCATION)) {
            MediaStore.setRequireOriginal(uri)
        } else {
            uri
        }

    private fun decode(uri: Uri, size: Int): Bitmap {
        val resolver = activity.contentResolver
        // Thumbnail sizes: the phone's own thumbnail cache, made once.
        if (Build.VERSION.SDK_INT >= 29 && size <= 512) {
            return resolver.loadThumbnail(uri, Size(size, size), null)
        }
        // Larger: the original, decoded at the size asked for, upright.
        if (Build.VERSION.SDK_INT >= 28) {
            val source = ImageDecoder.createSource(resolver, uri)
            return ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
                val (w, h) = Pair(info.size.width, info.size.height)
                val scale = minOf(1.0, size.toDouble() / maxOf(w, h))
                decoder.setTargetSize(maxOf(1, (w * scale).toInt()), maxOf(1, (h * scale).toInt()))
                // Software, so it can be compressed to JPEG.
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            }
        }
        return decodeOld(uri, size)
    }

    /** Android 8 and earlier: sampled down, then turned as MediaStore says. */
    private fun decodeOld(uri: Uri, size: Int): Bitmap {
        val resolver = activity.contentResolver
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) }
        var sample = 1
        while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= size) sample *= 2
        val opts = BitmapFactory.Options().apply { inSampleSize = sample }
        val bitmap = resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, opts) }
            ?: throw java.io.IOException("Couldn't read this photo.")
        var degrees = 0
        resolver.query(uri, arrayOf(MediaStore.Images.Media.ORIENTATION), null, null, null)?.use {
            if (it.moveToFirst()) degrees = it.getInt(0)
        }
        if (degrees == 0) return bitmap
        val turn = Matrix().apply { postRotate(degrees.toFloat()) }
        return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, turn, true)
    }
}
