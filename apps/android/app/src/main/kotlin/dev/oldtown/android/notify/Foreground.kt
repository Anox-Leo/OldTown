package dev.oldtown.android.notify

import android.app.ActivityManager

object Foreground {
    @Volatile
    var mainVisible: Boolean = false
}

fun shouldNotify(mainVisible: Boolean, processForeground: Boolean = false): Boolean =
    !mainVisible && !processForeground

fun processInForeground(): Boolean {
    val info = ActivityManager.RunningAppProcessInfo()
    ActivityManager.getMyMemoryState(info)
    return info.importance == ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
}

const val MAX_SHOWN_MESSAGES = 3

fun oldestBeyond(posted: List<Pair<Int, Long>>, max: Int): List<Int> =
    posted.sortedBy { it.second }.dropLast(max).map { it.first }
