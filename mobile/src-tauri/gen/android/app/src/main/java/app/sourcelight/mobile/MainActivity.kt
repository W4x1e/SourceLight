package app.sourcelight.mobile

import android.graphics.Color
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private val insets = Insets()

  override fun onCreate(savedInstanceState: Bundle?) {
    // The app is always dark, whatever the system's theme, so the status and
    // navigation bars always want light icons.
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
    )
    super.onCreate(savedInstanceState)
  }

  // Drawn edge to edge, the page has to keep clear of the status bar and the
  // gesture bar itself. Older Android WebViews report the top inset to CSS
  // (env(safe-area-inset-top)) but give 0 for the bottom, so the real ones
  // are handed over here: `SourceLightInsets.get()` returns "top,bottom" in
  // CSS pixels, and a `sourcelight-insets` event says they changed.
  //
  // Then "left,right": how far in from each side a swipe is the system's
  // Back gesture (0 with buttons). A drag that starts there never reaches the
  // page, so the editor keeps its sliders and crop grips clear of it.
  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(insets, "SourceLightInsets")
    ViewCompat.setOnApplyWindowInsetsListener(webView) { view, windowInsets ->
      val bars = windowInsets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val gestures = windowInsets.getInsets(WindowInsetsCompat.Type.systemGestures())
      val density = resources.displayMetrics.density
      insets.top = bars.top / density
      insets.bottom = bars.bottom / density
      insets.left = gestures.left / density
      insets.right = gestures.right / density
      (view as WebView).evaluateJavascript("window.dispatchEvent(new Event('sourcelight-insets'))", null)
      windowInsets
    }
  }

  class Insets {
    @Volatile var top = 0f
    @Volatile var bottom = 0f
    @Volatile var left = 0f
    @Volatile var right = 0f

    @JavascriptInterface
    fun get(): String = "$top,$bottom,$left,$right"
  }
}
