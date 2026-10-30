package app.sourcelight.track

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.os.Looper
import java.io.File
import java.io.FileWriter

/**
 * The recording: a foreground service asking Android's LocationManager for a
 * position every [EXTRA_INTERVAL] ms and appending each to [EXTRA_FILE].
 *
 * GPS always; the network provider too, but its fixes are only kept while GPS
 * has said nothing for a while (indoors, under a roof), as they are much
 * coarser. If Android kills the app, START_STICKY brings the service back and
 * it carries on into the same file.
 */
class TrackService : Service(), LocationListener {

    companion object {
        const val EXTRA_FILE = "file"
        const val EXTRA_INTERVAL = "intervalMs"
        private const val CHANNEL = "shoot_track"
        private const val NOTIFICATION = 7041
        private const val PREFS = "shoot_track"
        /** Network fixes are kept after this long without a GPS one. */
        private const val GPS_SILENCE_MS = 30_000L

        @Volatile var recording = false
        @Volatile var file: String? = null
        @Volatile var points = 0
        @Volatile var lastFixMs = 0L
        @Volatile var lastAccuracy = 0f
        @Volatile private var lastGpsMs = 0L

        fun savedFile(context: Context): String? =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(EXTRA_FILE, null)

        fun forget(context: Context) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().clear().apply()
            file = null
            points = 0
            lastFixMs = 0L
        }
    }

    private var writer: FileWriter? = null
    private var manager: LocationManager? = null

    override fun onBind(intent: Intent?): IBinder? = null

    @SuppressLint("MissingPermission") // checked by the plugin before starting
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val prefs = getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        // A restart after Android killed the app has no intent: carry on.
        val path = intent?.getStringExtra(EXTRA_FILE) ?: prefs.getString(EXTRA_FILE, null)
        val interval = intent?.getLongExtra(EXTRA_INTERVAL, 10_000L) ?: prefs.getLong(EXTRA_INTERVAL, 10_000L)
        if (path == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        prefs.edit().putString(EXTRA_FILE, path).putLong(EXTRA_INTERVAL, interval).apply()

        startInForeground()

        if (!recording) {
            val target = File(path)
            target.parentFile?.mkdirs()
            points = if (target.exists()) target.readLines().count { it.isNotBlank() } else 0
            writer = FileWriter(target, true)
            file = path
            val lm = getSystemService(Context.LOCATION_SERVICE) as LocationManager
            manager = lm
            for (provider in listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)) {
                if (lm.allProviders.contains(provider)) {
                    lm.requestLocationUpdates(provider, interval, 0f, this, Looper.getMainLooper())
                }
            }
            recording = true
        }
        return START_STICKY
    }

    override fun onLocationChanged(location: Location) {
        val gps = location.provider == LocationManager.GPS_PROVIDER
        if (gps) {
            lastGpsMs = System.currentTimeMillis()
        } else if (System.currentTimeMillis() - lastGpsMs < GPS_SILENCE_MS) {
            return
        }
        val w = writer ?: return
        val altitude = if (location.hasAltitude()) location.altitude.toString() else ""
        w.write("${location.time},${location.latitude},${location.longitude},${location.accuracy},$altitude\n")
        w.flush()
        points += 1
        lastFixMs = location.time
        lastAccuracy = location.accuracy
    }

    // Older Android calls these, and their defaults throw on some versions.
    @Deprecated("Deprecated in Java")
    override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
    override fun onProviderEnabled(provider: String) {}
    override fun onProviderDisabled(provider: String) {}

    override fun onDestroy() {
        manager?.removeUpdates(this)
        writer?.close()
        writer = null
        recording = false
        super.onDestroy()
    }

    private fun startInForeground() {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, "Shoot track", NotificationManager.IMPORTANCE_LOW)
            )
        }
        val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE)
        }
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL)
        } else {
            @Suppress("DEPRECATION") Notification.Builder(this)
        }
        val notification = builder
            .setContentTitle("Recording a track")
            .setContentText("SourceLight is noting where you are, for your photos.")
            .setSmallIcon(android.R.drawable.ic_menu_mylocation)
            .setOngoing(true)
            .setContentIntent(open)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION)
        } else {
            startForeground(NOTIFICATION, notification)
        }
    }
}
