package dev.oldtown.android

import dev.oldtown.android.ui.components.MARK_PATH
import dev.oldtown.android.ui.components.MARK_SMALL_PATH
import dev.oldtown.android.ui.components.WORDMARK_PATH
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Test

class BrandTest {
    private val branding = File("../../../branding")
    private val drawable = File("src/main/res/drawable")

    private fun paths(svg: String): List<String> =
        Regex(""" d="([^"]+)"""").findAll(File(branding, svg).readText()).map { it.groupValues[1] }.toList()

    @Test
    fun composeMarkAndLogoMatchBranding() {
        assertEquals(paths("archivolte-signe.svg"), listOf(MARK_PATH))
        assertEquals(paths("archivolte-petit.svg"), listOf(MARK_SMALL_PATH))
        assertEquals(paths("logotype-mono.svg"), listOf(MARK_PATH, WORDMARK_PATH))
    }

    private fun placed(svg: String): List<String> {
        val text = File(branding, svg).readText()
        val m = Regex("""transform="translate\(([\d.]+) ([\d.]+)\) scale\(([\d.]+)\) translate\(-([\d.]+) -([\d.]+)\)"""")
            .find(text) ?: error("placement absent de $svg")
        val (tx, ty, s, ox, oy) = m.destructured
        return listOf(s, s, tx, ty, "-$ox", "-$oy") + paths(svg)
    }

    private fun vector(xml: String): List<String> {
        val text = File(drawable, xml).readText()
        fun attr(name: String) = Regex("""android:$name="([^"]+)"""").findAll(text).map { it.groupValues[1] }.toList()
        return attr("scaleX") + attr("scaleY") + attr("translateX").take(1) + attr("translateY").take(1) +
            attr("translateX").drop(1) + attr("translateY").drop(1) + attr("pathData")
    }

    @Test
    fun androidIconsMatchBranding() {
        assertEquals(placed("android-foreground.svg"), vector("ic_launcher_foreground.xml"))
        assertEquals(placed("android-monochrome.svg"), vector("ic_launcher_monochrome.xml"))
        assertEquals(placed("android-notification.svg"), vector("ic_stat_oldtown.xml"))
    }
}
