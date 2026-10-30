package app.sourcelight.gallery

import android.Manifest
import android.app.Activity
import android.content.ContentValues
import android.media.MediaScannerConnection
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.IOException
import java.util.concurrent.Executors

@InvokeArg
class SaveArgs {
    /** The received file, in the app's own folder. */
    var path: String = ""
    /** The name it gets in the gallery. */
    var name: String = ""
    var mime: String = "image/jpeg"
}

/**
 * Saves photos sent from the desktop into the gallery (docs/MOBILE.md, "Send
 * to phone"), in Pictures/SourceLight. Android 10 and later add them through
 * MediaStore with no permission; 9 and earlier write the folder directly,
 * which needs storage access, asked for the first time.
 */
@TauriPlugin(
    permissions = [Permission(strings = [Manifest.permission.WRITE_EXTERNAL_STORAGE], alias = "storage")]
)
class GalleryPlugin(private val activity: Activity) : Plugin(activity) {
    /** One copy at a time, off the main thread: a full-size TIFF is big. */
    private val worker = Executors.newSingleThreadExecutor()

    @Command
    fun save(invoke: Invoke) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q &&
            getPermissionState("storage") != PermissionState.GRANTED
        ) {
            requestPermissionForAlias("storage", invoke, "storageAnswered")
            return
        }
        copyIn(invoke)
    }

    @PermissionCallback
    fun storageAnswered(invoke: Invoke) {
        if (getPermissionState("storage") == PermissionState.GRANTED) {
            copyIn(invoke)
        } else {
            invoke.reject("Storage access was not given, so photos can't be saved to the gallery.")
        }
    }

    private fun copyIn(invoke: Invoke) {
        val args = invoke.parseArgs(SaveArgs::class.java)
        worker.execute {
            try {
                val source = File(args.path)
                val saved = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    viaMediaStore(source, args)
                } else {
                    viaFolder(source, args)
                }
                invoke.resolve(JSObject().put("name", saved))
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /**
     * Android 10+: pending until the bytes are in, so no gallery shows half a
     * photo. Returns the name MediaStore gave it: `name (1)` when one was taken.
     */
    private fun viaMediaStore(source: File, args: SaveArgs): String {
        val resolver = activity.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, args.name)
            put(MediaStore.Images.Media.MIME_TYPE, args.mime)
            put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/SourceLight")
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        val collection = MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = resolver.insert(collection, values)
            ?: throw IOException("The gallery would not take ${args.name}.")
        try {
            val out = resolver.openOutputStream(uri)
                ?: throw IOException("The gallery would not take ${args.name}.")
            out.use { o -> source.inputStream().use { it.copyTo(o) } }
            values.clear()
            values.put(MediaStore.Images.Media.IS_PENDING, 0)
            resolver.update(uri, values, null, null)
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
        return resolver.query(uri, arrayOf(MediaStore.Images.Media.DISPLAY_NAME), null, null, null)
            ?.use { if (it.moveToFirst()) it.getString(0) else null }
            ?: args.name
    }

    /** Android 9 and earlier: into the folder, then tell the gallery. */
    @Suppress("DEPRECATION")
    private fun viaFolder(source: File, args: SaveArgs): String {
        val dir = File(
            Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES),
            "SourceLight"
        )
        if (!dir.isDirectory && !dir.mkdirs()) {
            throw IOException("Couldn't make the SourceLight folder in Pictures.")
        }
        val dest = unused(dir, args.name)
        source.copyTo(dest)
        MediaScannerConnection.scanFile(activity, arrayOf(dest.path), arrayOf(args.mime), null)
        return dest.name
    }

    /** `name`, or `name (2)` and so on: both kept rather than one replaced. */
    private fun unused(dir: File, name: String): File {
        val first = File(dir, name)
        if (!first.exists()) return first
        val dot = name.lastIndexOf('.')
        val stem = if (dot > 0) name.substring(0, dot) else name
        val ext = if (dot > 0) name.substring(dot) else ""
        var n = 2
        while (true) {
            val candidate = File(dir, "$stem ($n)$ext")
            if (!candidate.exists()) return candidate
            n++
        }
    }
}
