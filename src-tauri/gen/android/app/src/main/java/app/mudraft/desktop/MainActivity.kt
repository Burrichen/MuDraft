package app.mudraft.desktop

import android.graphics.Color
import android.os.Bundle
import android.view.View
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  // The system Back button walks back through the app's pages (hash-routed history) and
  // only leaves the app from the first page.
  override val handleBackNavigation: Boolean = true

  override fun onCreate(savedInstanceState: Bundle?) {
    // The interface is always dark (color-scheme: dark), so the system bars are too:
    // transparent over the app's background, with light icons.
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
    )
    super.onCreate(savedInstanceState)
    // Android 15+ draws edge to edge. Keep the page out from under the status bar, display
    // cutout, and gesture bar, and lift it above the keyboard so inputs stay visible.
    val content = findViewById<View>(android.R.id.content)
    content.setBackgroundColor(Color.parseColor("#1E1E1E")) // --bg in theme/tokens.css
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, keyboard.bottom))
      WindowInsetsCompat.CONSUMED
    }
  }
}
