package io.github.mdreader.MdReader

import android.app.Activity
import android.content.ContentUris
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.core.content.IntentCompat
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.util.concurrent.Executors

@InvokeArg
class UriArgs {
  lateinit var uri: String
}

@InvokeArg
class WriteArgs {
  lateinit var uri: String
  lateinit var text: String
}

/**
 * Documents as Android hands them over: `content://` URIs from other apps
 * ("Open with", "Share") and from the system pickers. Reads and writes them,
 * and gives the file path behind a URI when there is one the app may read,
 * which the app prefers (relative images, links, folders, watching).
 *
 * Registered from Rust (`src/android.rs`); the page reaches it only through
 * the app's own commands.
 */
@TauriPlugin
class FilesPlugin(private val activity: Activity) : Plugin(activity) {
  private var webView: WebView? = null
  private val pending = mutableListOf<JSObject>()
  private val io = Executors.newSingleThreadExecutor()

  override fun load(webView: WebView) {
    super.load(webView)
    this.webView = webView
    collect(activity.intent)
  }

  override fun onNewIntent(intent: Intent) {
    if (collect(intent)) {
      // The page asks for them (take_opened) when told.
      webView?.post { webView?.evaluateJavascript("window.mdrOpened && window.mdrOpened()", null) }
    }
  }

  /** Keep the documents an intent brings. True when there were any. */
  private fun collect(intent: Intent?): Boolean {
    if (intent == null) return false
    // Reopened from Recents: the document was handled when it first came.
    if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return false
    val uris = mutableListOf<Uri>()
    when (intent.action) {
      Intent.ACTION_VIEW, Intent.ACTION_EDIT -> intent.data?.let { uris += it }
      Intent.ACTION_SEND ->
        IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let { uris += it }
      Intent.ACTION_SEND_MULTIPLE ->
        IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let { uris += it }
    }
    if (uris.isEmpty()) return false
    intent.action = Intent.ACTION_MAIN // handled
    synchronized(pending) { uris.forEach { pending += item(it) } }
    return true
  }

  private fun item(uri: Uri): JSObject {
    val o = JSObject()
    o.put("uri", uri.toString())
    o.put("path", pathOf(uri))
    return o
  }

  private fun items(list: List<JSObject>): JSObject {
    val r = JSObject()
    r.put("items", JSArray(list))
    return r
  }

  @Command
  fun pending(invoke: Invoke) {
    val taken = synchronized(pending) { pending.toList().also { pending.clear() } }
    invoke.resolve(items(taken))
  }

  // ------------------------------------------------------------- reading

  @Command
  fun read(invoke: Invoke) {
    val args = invoke.parseArgs(UriArgs::class.java)
    io.execute {
      try {
        val uri = Uri.parse(args.uri)
        val bytes = activity.contentResolver.openInputStream(uri)?.use { input ->
          val buf = input.readNBytesCompat(MAX_BYTES + 1)
          if (buf.size > MAX_BYTES) throw Failure("it is larger than ${MAX_BYTES / 1024 / 1024} MB")
          buf
        } ?: throw Failure("it could not be opened")
        // A picture or an archive would open as a page of noise.
        for (i in 0 until minOf(bytes.size, 8192)) if (bytes[i] == 0.toByte()) throw Failure("it is not a text file")
        var lossy = false
        val text = try {
          Charsets.UTF_8.newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(bytes)).toString()
        } catch (_: CharacterCodingException) {
          lossy = true
          String(bytes, Charsets.UTF_8)
        }
        val r = JSObject()
        r.put("name", displayName(uri))
        r.put("text", text)
        r.put("lossy", lossy)
        invoke.resolve(r)
      } catch (e: Failure) {
        invoke.reject(e.message)
      } catch (e: SecurityException) {
        invoke.reject("the app may no longer read it")
      } catch (e: Exception) {
        invoke.reject(e.message ?: "it could not be read")
      }
    }
  }

  @Command
  fun write(invoke: Invoke) {
    val args = invoke.parseArgs(WriteArgs::class.java)
    io.execute {
      try {
        val uri = Uri.parse(args.uri)
        // "wt" truncates; some providers only know "rwt".
        val out = try {
          activity.contentResolver.openOutputStream(uri, "wt")
        } catch (_: Exception) {
          activity.contentResolver.openOutputStream(uri, "rwt")
        } ?: throw Failure("it could not be opened for writing")
        out.use { it.write(args.text.toByteArray(Charsets.UTF_8)) }
        invoke.resolve()
      } catch (e: Failure) {
        invoke.reject(e.message)
      } catch (e: SecurityException) {
        invoke.reject("the app that shared this document only allowed reading it")
      } catch (e: Exception) {
        invoke.reject(e.message ?: "it could not be written")
      }
    }
  }

  private fun displayName(uri: Uri): String {
    try {
      activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
        if (c.moveToFirst()) c.getString(0)?.let { if (it.isNotBlank()) return it }
      }
    } catch (_: Exception) {}
    return uri.lastPathSegment?.substringAfterLast('/')?.ifBlank { null } ?: "Document"
  }

  // -------------------------------------------------------------- pickers

  @Command
  fun pickFiles(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      // Many apps and file systems don't know Markdown's type; the app
      // turns away what isn't text when it reads it.
      type = "*/*"
      putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
    }
    startActivityForResult(invoke, intent, "pickFilesResult")
  }

  @ActivityCallback
  fun pickFilesResult(invoke: Invoke, result: ActivityResult) {
    val data = result.data
    if (result.resultCode != Activity.RESULT_OK || data == null) {
      invoke.resolve(items(emptyList()))
      return
    }
    val uris = mutableListOf<Uri>()
    val clip = data.clipData
    if (clip != null) for (i in 0 until clip.itemCount) uris += clip.getItemAt(i).uri
    else data.data?.let { uris += it }
    uris.forEach(::keepAccess) // so the documents reopen at the next start
    invoke.resolve(items(uris.map(::item)))
  }

  @Command
  fun pickFolder(invoke: Invoke) {
    startActivityForResult(invoke, Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), "pickFolderResult")
  }

  @ActivityCallback
  fun pickFolderResult(invoke: Invoke, result: ActivityResult) {
    val r = JSObject()
    val uri = result.data?.data
    r.put("item", if (result.resultCode == Activity.RESULT_OK && uri != null) item(uri) else null)
    invoke.resolve(r)
  }

  private fun keepAccess(uri: Uri) {
    val rw = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
    try {
      activity.contentResolver.takePersistableUriPermission(uri, rw)
    } catch (_: Exception) {
      try {
        activity.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
      } catch (_: Exception) {}
    }
  }

  // ---------------------------------------------------- all files access

  @Command
  fun allFiles(invoke: Invoke) {
    invoke.resolve(granted())
  }

  @Command
  fun requestAllFiles(invoke: Invoke) {
    if (Environment.isExternalStorageManager()) {
      invoke.resolve(granted())
      return
    }
    val forApp = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:${activity.packageName}"))
    val intent = if (forApp.resolveActivity(activity.packageManager) != null) forApp
      else Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
    startActivityForResult(invoke, intent, "allFilesResult")
  }

  @ActivityCallback
  fun allFilesResult(invoke: Invoke, @Suppress("UNUSED_PARAMETER") result: ActivityResult) {
    invoke.resolve(granted())
  }

  private fun granted(): JSObject {
    val r = JSObject()
    r.put("granted", Environment.isExternalStorageManager())
    return r
  }

  @Command
  fun resolve(invoke: Invoke) {
    val args = invoke.parseArgs(UriArgs::class.java)
    io.execute {
      val r = JSObject()
      r.put("path", pathOf(Uri.parse(args.uri)))
      invoke.resolve(r)
    }
  }

  @Command
  fun moveToBack(invoke: Invoke) {
    invoke.resolve()
    activity.runOnUiThread { activity.moveTaskToBack(true) }
  }

  // ---------------------------------------------------------------- paths

  /** The file behind `uri`, if it has one this app may read. */
  private fun pathOf(uri: Uri): String? {
    val path = when (uri.scheme) {
      "file" -> uri.path
      "content" -> contentPath(uri)
      else -> null
    } ?: return null
    val f = File(path)
    return if (f.canRead()) f.absolutePath else null
  }

  private fun contentPath(uri: Uri): String? {
    try {
      val tree = DocumentsContract.isTreeUri(uri) && !DocumentsContract.isDocumentUri(activity, uri)
      val id = when {
        tree -> DocumentsContract.getTreeDocumentId(uri)
        DocumentsContract.isDocumentUri(activity, uri) -> DocumentsContract.getDocumentId(uri)
        else -> null
      }
      if (id != null) when (uri.authority) {
        // "primary:Documents/notes.md", or "1A2B-3C4D:…" on a card
        "com.android.externalstorage.documents" -> {
          val volume = id.substringBefore(':')
          val rest = id.substringAfter(':', "")
          val root = if (volume.equals("primary", true)) Environment.getExternalStorageDirectory().path else "/storage/$volume"
          return if (rest.isEmpty()) root else "$root/$rest"
        }
        "com.android.providers.downloads.documents" -> {
          if (id.startsWith("raw:")) return id.removePrefix("raw:")
          if (id.startsWith("msf:")) id.removePrefix("msf:").toLongOrNull()?.let { n ->
            return dataColumn(ContentUris.withAppendedId(MediaStore.Files.getContentUri("external"), n))
          }
        }
      }
    } catch (_: Exception) {}
    // Providers that still report a path: MediaStore, many file managers.
    return dataColumn(uri)
  }

  private fun dataColumn(uri: Uri): String? {
    try {
      activity.contentResolver.query(uri, arrayOf(MediaStore.MediaColumns.DATA), null, null, null)?.use { c ->
        if (c.moveToFirst()) {
          val i = c.getColumnIndex(MediaStore.MediaColumns.DATA)
          if (i >= 0) c.getString(i)?.let { if (it.startsWith("/")) return it }
        }
      }
    } catch (_: Exception) {}
    return null
  }

  private class Failure(message: String) : Exception(message)

  companion object {
    private const val MAX_BYTES = 32 * 1024 * 1024
  }
}

/** InputStream.readNBytes is API 33+. */
private fun java.io.InputStream.readNBytesCompat(limit: Int): ByteArray {
  val out = java.io.ByteArrayOutputStream()
  val buf = ByteArray(64 * 1024)
  while (out.size() < limit) {
    val n = read(buf, 0, minOf(buf.size, limit - out.size()))
    if (n < 0) break
    out.write(buf, 0, n)
  }
  return out.toByteArray()
}
