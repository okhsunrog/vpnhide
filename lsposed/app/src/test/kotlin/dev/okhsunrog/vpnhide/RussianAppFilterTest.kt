package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.isRussianApp
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RussianAppFilterTest {
    // ── Package prefix "ru." ──

    @Test
    fun `ru dot prefix is Russian`() {
        assertTrue(isRussianApp("ru.nspk.mirpay", "Mir Pay"))
        assertTrue(isRussianApp("ru.gosuslugi.app", "Gosuslugi"))
        assertTrue(isRussianApp("ru.sberbank.sberbankid", "Sber ID"))
        assertTrue(isRussianApp("ru.mail.cloud", "Mail.ru Cloud"))
    }

    // ── Known Russian company prefixes ──

    @Test
    fun `Sberbank is Russian`() {
        assertTrue(isRussianApp("com.sberbank.sberbankid", "Sber ID"))
    }

    @Test
    fun `Tinkoff is Russian`() {
        assertTrue(isRussianApp("com.tinkoff.investing", "Tinkoff Investing"))
        assertTrue(isRussianApp("com.tcsbank.c2c", "T-Bank"))
    }

    @Test
    fun `Yandex is Russian`() {
        assertTrue(isRussianApp("com.yandex.browser", "Yandex Browser"))
        assertTrue(isRussianApp("com.yandex.maps", "Yandex Maps"))
    }

    @Test
    fun `VK is Russian`() {
        assertTrue(isRussianApp("com.vkontakte.android", "VK"))
        assertTrue(isRussianApp("com.vk.im", "VK Messenger"))
    }

    @Test
    fun `other known Russian companies`() {
        assertTrue(isRussianApp("com.kaspersky.security.cloud", "Kaspersky"))
        assertTrue(isRussianApp("com.wildberries.client", "Wildberries"))
        assertTrue(isRussianApp("io.ozon.android", "Ozon"))
        assertTrue(isRussianApp("com.avito.android", "Avito"))
        assertTrue(isRussianApp("com.alfabank.mobile.android", "Alfa-Bank"))
        assertTrue(isRussianApp("me.sovcombank.halva", "Halva"))
        assertTrue(isRussianApp("com.drweb.pro", "Dr.Web"))
        assertTrue(isRussianApp("com.punicapp.whoosh", "Whoosh"))
        assertTrue(isRussianApp("com.mtsbank.app", "MTS Bank"))
        assertTrue(isRussianApp("com.rosbank.android", "Rosbank"))
        assertTrue(isRussianApp("com.raiffeisen.rmobile", "Raiffeisen"))
        assertTrue(isRussianApp("com.kontur.extern", "Kontur"))
    }

    @Test
    fun `batch 2026-04 Russian apps are matched`() {
        // retailers / marketplaces / food
        assertTrue(isRussianApp("com.allgoritm.youla", "Youla"))
        assertTrue(isRussianApp("com.deliveryclub", "Delivery Club"))
        assertTrue(isRussianApp("com.edadeal.android", "Edadeal"))
        assertTrue(isRussianApp("com.kazanexpress.ke_app", "KazanExpress"))
        assertTrue(isRussianApp("com.zolla.app", "Zolla"))
        assertTrue(isRussianApp("www.metro.com", "Metro"))
        // couriers / logistics
        assertTrue(isRussianApp("com.logistic.sdek", "SDEK"))
        assertTrue(isRussianApp("com.octopod.russianpost.client.android", "Russian Post"))
        assertTrue(isRussianApp("com.taxsee.taxsee", "Taxsee"))
        // telecom / smart home (prefix catches both ER-Telecom apps)
        assertTrue(isRussianApp("com.ertelecom.agent", "Dom.ru"))
        assertTrue(isRussianApp("com.ertelecom.smarthome", "Dom.ru Smart Home"))
        // multi-app aggregators (prefix catches all platfomni pharmacies)
        assertTrue(isRussianApp("com.platfomni.gorzdrav", "Gorzdrav"))
        assertTrue(isRussianApp("com.platfomni.vita", "Vita"))
        assertTrue(isRussianApp("com.platfomni.asna", "ASNA"))
        // corporate / government-adjacent
        assertTrue(isRussianApp("com.consultantplus.hs", "ConsultantPlus"))
        assertTrue(isRussianApp("com.greenatom.atomspace", "AtomSpace"))
        // food chains
        assertTrue(isRussianApp("com.apegroup.mcdonaldsrussia", "McDonald's Russia"))
        assertTrue(isRussianApp("com.FoodSoul.KurskRollStreet", "Kursk Roll Street"))
        assertTrue(isRussianApp("com.rubeacon.tashirpizza", "Tashir Pizza"))
        // misc one-offs
        assertTrue(isRussianApp("club.chizhik", "Chizhik"))
        assertTrue(isRussianApp("one.belousov.wordgame", "Word Game"))
        assertTrue(isRussianApp("starter.pokemoon.client", "Pokemoon"))
        assertTrue(isRussianApp("vesnasoft.teleform", "Teleform"))
    }

    @Test
    fun `batch 2026-04-19 4pda feedback is matched`() {
        assertTrue(isRussianApp("air.ru.samis.Pribyvalka63", "Прибывалка 63"))
        assertTrue(isRussianApp("info.goodline.events", "Кавёр"))
        assertTrue(isRussianApp("com.rosdomofon.rdua", "РосДомофон"))
        assertTrue(isRussianApp("com.drivee.taxi.rides", "Drivee"))
        assertTrue(isRussianApp("com.setka", "Сетка"))
        assertTrue(isRussianApp("com.twinby", "Twinby"))
    }

    @Test
    fun `batch 2026-04-20 brands are matched`() {
        assertTrue(isRussianApp("com.swiftsoft.anixartd", "Anixart"))
        assertTrue(isRussianApp("bz.epn.cashback.epncashback", "ePN Cashback"))
        assertTrue(isRussianApp("gpm.tnt_premier", "ТНТ Premier"))
        assertTrue(isRussianApp("com.cardsmobile.swoo", "Swoo"))
        assertTrue(isRussianApp("today.maxi.mobile", "Макси"))
        assertTrue(isRussianApp("com.dartit.RTcabinet", "Ростелеком Личный кабинет"))
        assertTrue(isRussianApp("com.warefly.checkscan", "Проверка чеков ФНС"))
    }

    // ── Non-Russian apps (must NOT match) ──

    @Test
    fun `Telegram is NOT matched`() {
        assertFalse(isRussianApp("org.telegram.messenger", "Telegram"))
    }

    @Test
    fun `Google apps are not Russian`() {
        assertFalse(isRussianApp("com.google.android.apps.maps", "Google Maps"))
        assertFalse(isRussianApp("com.google.android.gm", "Gmail"))
        assertFalse(isRussianApp("com.google.android.apps.photos", "Фото"))
        assertFalse(isRussianApp("com.google.android.apps.docs", "Диск"))
    }

    @Test
    fun `localized system apps are not Russian`() {
        assertFalse(isRussianApp("android.autoinstalls.config.google.nexus", "Конфигурация"))
        assertFalse(isRussianApp("com.android.calendar", "Календарь"))
    }

    @Test
    fun `international apps are not Russian`() {
        assertFalse(isRussianApp("com.whatsapp", "WhatsApp"))
        assertFalse(isRussianApp("com.instagram.android", "Instagram"))
        assertFalse(isRussianApp("com.spotify.music", "Spotify"))
        assertFalse(isRussianApp("com.netflix.mediaclient", "Netflix"))
    }

    @Test
    fun `Samsung apps are not Russian`() {
        assertFalse(isRussianApp("com.samsung.android.calendar", "Samsung Calendar"))
    }

    // ── Edge cases ──

    @Test
    fun `empty label and non-Russian package`() {
        assertFalse(isRussianApp("com.example.app", ""))
    }

    @Test
    fun `Cyrillic-only label without Russian package does NOT match`() {
        assertFalse(isRussianApp("com.example.app", "Госуслуги"))
    }
}
