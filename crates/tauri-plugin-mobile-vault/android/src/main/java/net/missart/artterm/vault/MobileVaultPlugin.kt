package net.missart.artterm.vault

import android.app.Activity
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class OperatorBytesArgs {
    var context: IntArray = intArrayOf()
    var data: IntArray = intArrayOf()
}

@TauriPlugin
class MobileVaultPlugin(private val activity: Activity) : Plugin(activity) {
    // Independent operator namespace and no-backup files. Existing
    // unrelated app data is left untouched.
    private val operatorAlias = "art-term-openab-operator-wrap-v1"
    private val operatorMarkerText = "art-term-openab-operator-v1"
    private val operatorMarkerBytes get() = operatorMarkerText.toByteArray(Charsets.US_ASCII)
    private val operatorRoot: File get() = File(activity.noBackupFilesDir.canonicalFile, "openab-operator-v1")
    private val operatorMarker: File get() = File(operatorRoot, "key-used-v1")

    private fun operatorKey(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(operatorAlias, null) as? SecretKey)?.let { return it }
        // The marker survives deleting all Hosts. Keystore loss must not silently
        // create a replacement, even with an empty envelope or absent Host file.
        require(create && !operatorMarker.exists()
            && !File(operatorRoot, "connections.toml").exists()
            && !File(operatorRoot, "runtime-hosts-v1.json").exists())
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(KeyGenParameterSpec.Builder(operatorAlias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true).setKeySize(256).build())
        return generator.generateKey()
    }

    @Command
    @Synchronized
    fun operatorInitialize(invoke: Invoke) {
        try {
            require(operatorRoot.mkdirs() || operatorRoot.isDirectory)
            require(operatorRoot.canonicalFile == operatorRoot.absoluteFile)
            require(operatorMarker.canonicalFile == operatorMarker.absoluteFile)
            operatorKey(true)
            if (!operatorMarker.exists()) {
                java.io.FileOutputStream(operatorMarker).use {
                    it.write(operatorMarkerBytes); it.fd.sync()
                }
            }
            require(operatorMarker.isFile && operatorMarker.length() == operatorMarkerBytes.size.toLong())
            require(operatorMarker.readText(Charsets.US_ASCII) == operatorMarkerText)
            val result = JSObject(); result.put("path", operatorRoot.absolutePath); invoke.resolve(result)
        } catch (_: Exception) { invoke.reject("Unable to initialize operator vault") }
    }

    private fun operatorBytes(values: IntArray, limit: Int): ByteArray {
        require(values.isNotEmpty() && values.size <= limit && values.all { it in 0..255 })
        return ByteArray(values.size) { values[it].toByte() }
    }

    private fun operatorCipher(invoke: Invoke, encrypt: Boolean) {
        var data: ByteArray? = null
        var context: ByteArray? = null
        var output: ByteArray? = null
        var args: OperatorBytesArgs? = null
        try {
            args = invoke.parseArgs(OperatorBytesArgs::class.java)
            data = operatorBytes(args.data, if (encrypt) 1024 else 1053)
            context = operatorBytes(args.context, 4096)
            require(operatorMarker.isFile && operatorMarker.length() == operatorMarkerBytes.size.toLong() && operatorMarker.canonicalFile == operatorMarker.absoluteFile)
            require(operatorMarker.readText(Charsets.US_ASCII) == operatorMarkerText)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            if (encrypt) cipher.init(Cipher.ENCRYPT_MODE, operatorKey(false))
            else {
                require(data.size in 30..1053 && data[0] == 1.toByte())
                cipher.init(Cipher.DECRYPT_MODE, operatorKey(false), GCMParameterSpec(128, data.copyOfRange(1, 13)))
            }
            cipher.updateAAD("net.missart.artterm:openab-operator:v1:".toByteArray(Charsets.US_ASCII))
            cipher.updateAAD(context)
            output = if (encrypt) {
                require(cipher.iv.size == 12)
                byteArrayOf(1) + cipher.iv + cipher.doFinal(data)
            } else cipher.doFinal(data, 13, data.size - 13)
            require(encrypt || output.size in 1..1024)
            val array = org.json.JSONArray()
            output.forEach { array.put(it.toInt() and 255) }
            val result = JSObject(); result.put("data", array); invoke.resolve(result)
        } catch (_: Exception) { invoke.reject("Unable to protect operator credential") }
        finally {
            data?.fill(0); context?.fill(0); output?.fill(0)
            args?.data?.fill(0); args?.context?.fill(0)
        }
    }

    @Command
    @Synchronized
    fun operatorSeal(invoke: Invoke) = operatorCipher(invoke, true)

    @Command
    @Synchronized
    fun operatorUnseal(invoke: Invoke) = operatorCipher(invoke, false)

}
