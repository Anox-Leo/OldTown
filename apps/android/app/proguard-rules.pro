# Liaisons UniFFI (JNA) : classes appelées par réflexion depuis le natif.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class dev.oldtown.core.** { *; }
-dontwarn java.awt.**
# zxing-cpp : méthodes natives et classes construites par JNI.
-keep class zxingcpp.** { *; }
