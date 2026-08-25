package dev.okhsunrog.vpnhide.picker

/**
 * Known Russian company package prefixes that don't start with "ru.".
 * Sorted alphabetically for readability. Each entry is matched via startsWith().
 */
private val KNOWN_RUSSIAN_PREFIXES =
    listOf(
        "air.ru.samis.Pribyvalka63",
        "bz.epn",
        "club.chizhik",
        "com.alfabank",
        "com.allgoritm.youla",
        "com.apegroup.mcdonaldsrussia",
        "com.apteka.sklad",
        "com.avito",
        "com.cardsmobile",
        "com.consultantplus",
        "com.dartit",
        "com.deliveryclub",
        "com.drivee.taxi",
        "com.drweb",
        "com.edadeal",
        "com.ertelecom",
        "com.FoodSoul",
        "com.greenatom",
        "com.hintsolutions.donor",
        "com.idamob.tinkoff",
        "com.it.coffeelike",
        "com.kaspersky",
        "com.kazanexpress",
        "com.kontur",
        "com.logistic.sdek",
        "com.ltech.rusap",
        "com.mtsbank",
        "com.nizhniy_mobile",
        "com.octopod.russianpost",
        "com.platfomni",
        "com.prorodinki",
        "com.punicapp.whoosh",
        "com.quantron.sushimarket",
        "com.raiffeisen",
        "com.rosbank",
        "com.rosdomofon",
        "com.rubeacon.tashirpizza",
        "com.rusdev",
        "com.sberbank",
        "com.setka",
        "com.swiftsoft",
        "com.taxsee",
        "com.tcsbank",
        "com.tinkoff",
        "com.twinby",
        "com.vk.",
        "com.vkontakte",
        "com.warefly",
        "com.wildberries",
        "com.yandex",
        "com.zolla",
        "gb.sweetlifecl",
        "gpm.tnt_premier",
        "info.goodline",
        "io.ozon",
        "me.sovcombank",
        "one.belousov.wordgame",
        "starter.pokemoon.client",
        "today.maxi",
        "vesnasoft.teleform",
        "www.metro.com",
    )

/**
 * Detect Russian apps by package name.
 *
 * Checks:
 * 1. Package starts with "ru." — strong signal (ru.nspk.mirpay, ru.gosuslugi, etc.)
 * 2. Package matches a known Russian company prefix
 *
 * Does NOT use Cyrillic label detection — too many false positives from
 * localized international apps (Google, Samsung, etc.).
 */
fun isRussianApp(
    packageName: String,
    @Suppress("UNUSED_PARAMETER") label: String,
): Boolean {
    if (packageName.startsWith("ru.")) return true
    if (KNOWN_RUSSIAN_PREFIXES.any { packageName.startsWith(it) }) return true
    return false
}
