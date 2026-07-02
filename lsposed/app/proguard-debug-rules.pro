# Keep debug stack traces readable while still letting R8 shrink the default
# debug APK enough for LSPosed/Vector cold-start smoke tests.
-dontoptimize
-dontobfuscate
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
