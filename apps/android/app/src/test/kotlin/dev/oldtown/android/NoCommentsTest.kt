package dev.oldtown.android

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Test

class NoCommentsTest {
    private val generated = setOf(
        "src/main/kotlin/dev/oldtown/android/ui/theme/Tokens.kt",
        "src/main/res/values/tokens.xml",
        "src/main/res/values-night/tokens.xml",
    )

    private val roots = listOf(
        "src",
        "build.gradle.kts",
        "proguard-rules.pro",
        "../build.gradle.kts",
        "../settings.gradle.kts",
        "../gradle.properties",
        "../gradle/libs.versions.toml",
    )

    private val extensions = setOf("kt", "kts", "xml", "toml", "properties", "pro")

    @Test
    fun noCommentsOutsideGeneratedFiles() {
        val found = roots.map(::File).filter { it.exists() }
            .flatMap { root -> root.walkTopDown().filter { it.isFile && it.extension in extensions }.toList() }
            .filter { it.invariantSeparatorsPath !in generated }
            .flatMap { f -> commentLines(f.extension, f.readText()).map { "${f.invariantSeparatorsPath}:$it" } }
        assertEquals(emptyList<String>(), found)
    }

    @Test
    fun theLexerSeesCommentsButNotStrings() {
        val src = listOf(
            "val url = \"wss://relais:7878\"",
            "val raw = \"\"\"a // b \${x + \"/* c */\"}\"\"\"",
            "val q = '\"'",
            "val s = '\\''",
            "val t = \"\${if (a) \"{\" else \"}\"}\"",
            "/* un /* imbriqué */ toujours */ val u = 1",
            "val v = 2 // fin",
        ).joinToString("\n")
        assertEquals(listOf(6, 7), commentLines("kt", src))
        assertEquals(listOf(2), commentLines("xml", "<a>\n<!-- x -->\n<b c=\"d\"/></a>"))
        assertEquals(listOf(1), commentLines("toml", "# x\n[versions]\nurl = \"a#b\""))
    }
}

internal fun commentLines(extension: String, src: String): List<Int> {
    val starts = when (extension) {
        "kt", "kts" -> kotlinComments(src)
        "xml" -> Regex("<!--").findAll(src).map { it.range.first }.toList()
        else -> hashComments(src)
    }
    return starts.map { at -> src.substring(0, at).count { it == '\n' } + 1 }
}

private fun hashComments(src: String): List<Int> {
    val starts = mutableListOf<Int>()
    var pos = 0
    for (line in src.split('\n')) {
        val body = line.trimStart()
        if (body.startsWith("#") || body.startsWith("!")) starts += pos + line.length - body.length
        pos += line.length + 1
    }
    return starts
}

private fun kotlinComments(src: String): List<Int> {
    val starts = mutableListOf<Int>()
    val stack = ArrayDeque<IntArray>()
    stack.addLast(intArrayOf(CODE, 0))
    val n = src.length
    var i = 0
    while (i < n) {
        val state = stack.last()
        val c = src[i]
        when (state[0]) {
            CODE -> when {
                src.startsWith("//", i) -> {
                    starts += i
                    val end = src.indexOf('\n', i)
                    i = if (end < 0) n else end
                }
                src.startsWith("/*", i) -> {
                    starts += i
                    var depth = 1
                    var j = i + 2
                    while (j < n && depth > 0) {
                        when {
                            src.startsWith("/*", j) -> {
                                depth++
                                j += 2
                            }
                            src.startsWith("*/", j) -> {
                                depth--
                                j += 2
                            }
                            else -> j++
                        }
                    }
                    i = j
                }
                src.startsWith("\"\"\"", i) -> {
                    stack.addLast(intArrayOf(RAW, 0))
                    i += 3
                }
                c == '"' -> {
                    stack.addLast(intArrayOf(STRING, 0))
                    i++
                }
                c == '\'' -> {
                    var j = i + 1
                    if (j < n && src[j] == '\\') {
                        j += 2
                        while (j < n && src[j] != '\'') j++
                    } else {
                        j++
                    }
                    i = j + 1
                }
                c == '{' -> {
                    state[1]++
                    i++
                }
                c == '}' -> {
                    if (stack.size > 1 && state[1] == 0) stack.removeLast() else state[1]--
                    i++
                }
                else -> i++
            }
            STRING -> when {
                c == '\\' -> i += 2
                c == '"' -> {
                    stack.removeLast()
                    i++
                }
                src.startsWith("\${", i) -> {
                    stack.addLast(intArrayOf(CODE, 0))
                    i += 2
                }
                else -> i++
            }
            else -> when {
                src.startsWith("\"\"\"", i) -> {
                    var j = i + 3
                    while (j < n && src[j] == '"') j++
                    stack.removeLast()
                    i = j
                }
                src.startsWith("\${", i) -> {
                    stack.addLast(intArrayOf(CODE, 0))
                    i += 2
                }
                else -> i++
            }
        }
    }
    return starts
}

private const val CODE = 0
private const val STRING = 1
private const val RAW = 2
