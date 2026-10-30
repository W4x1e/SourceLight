package app.sourcelight.qrscan

import android.Manifest
import android.app.Activity
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions

@InvokeArg
class ScanArgs {
    /** The line shown under the viewfinder. */
    var prompt: String = ""
}

/**
 * Scans a QR code with the camera (docs/MOBILE.md, "Pairing"), on ZXing's
 * scanner screen. Resolves `{ content }`, or `{ content: null }` when the
 * person backs out. Camera permission uses the base class's
 * `requestPermissions` with the alias below.
 */
@TauriPlugin(
    permissions = [Permission(strings = [Manifest.permission.CAMERA], alias = "camera")]
)
class QrScanPlugin(private val activity: Activity) : Plugin(activity) {

    @Command
    fun scan(invoke: Invoke) {
        val args = invoke.parseArgs(ScanArgs::class.java)
        val options = ScanOptions()
            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            .setPrompt(args.prompt)
            .setBeepEnabled(false)
            .setOrientationLocked(false)
        startActivityForResult(invoke, options.createScanIntent(activity), "scanned")
    }

    @ActivityCallback
    fun scanned(invoke: Invoke, result: ActivityResult) {
        val scan = ScanIntentResult.parseActivityResult(result.resultCode, result.data)
        val out = JSObject()
        out.put("content", scan.contents)
        invoke.resolve(out)
    }
}
