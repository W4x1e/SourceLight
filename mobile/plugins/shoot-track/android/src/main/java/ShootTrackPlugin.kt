package app.sourcelight.track

import android.Manifest
import android.app.Activity
import android.content.Intent
import androidx.core.content.ContextCompat
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class StartArgs {
    /** Where to append the points: one `epochMillis,lat,lon,accuracy,altitude` line each. */
    lateinit var file: String
    /** How often to ask for a position. */
    var intervalMs: Long = 10_000
}

/**
 * Records a GPS track while the screen is off (docs/MOBILE.md, "Shoot
 * track"). The app's Rust core calls these commands; the recording itself is
 * [TrackService], a foreground service, so it outlives the app being in the
 * background. Permissions use the base class's `checkPermissions` and
 * `requestPermissions` with the aliases below.
 */
@TauriPlugin(
    permissions = [
        Permission(
            strings = [Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION],
            alias = "location"
        ),
        Permission(strings = [Manifest.permission.POST_NOTIFICATIONS], alias = "notifications"),
    ]
)
class ShootTrackPlugin(private val activity: Activity) : Plugin(activity) {

    @Command
    fun start(invoke: Invoke) {
        val args = invoke.parseArgs(StartArgs::class.java)
        // Android 14 refuses to start a location service without it.
        if (getPermissionState("location") != PermissionState.GRANTED) {
            invoke.reject("Location permission is needed to record a track.")
            return
        }
        val intent = Intent(activity, TrackService::class.java)
            .putExtra(TrackService.EXTRA_FILE, args.file)
            .putExtra(TrackService.EXTRA_INTERVAL, args.intervalMs)
        ContextCompat.startForegroundService(activity, intent)
        val s = status()
        s.put("recording", true)
        s.put("file", args.file)
        invoke.resolve(s)
    }

    @Command
    fun stop(invoke: Invoke) {
        val result = status()
        activity.stopService(Intent(activity, TrackService::class.java))
        TrackService.forget(activity)
        result.put("recording", false)
        invoke.resolve(result)
    }

    @Command
    fun status(invoke: Invoke) {
        invoke.resolve(status())
    }

    private fun status(): JSObject {
        val s = JSObject()
        s.put("recording", TrackService.recording)
        s.put("file", TrackService.file ?: TrackService.savedFile(activity))
        s.put("points", TrackService.points)
        s.put("lastFixMs", TrackService.lastFixMs)
        s.put("lastAccuracy", TrackService.lastAccuracy.toDouble())
        return s
    }
}
