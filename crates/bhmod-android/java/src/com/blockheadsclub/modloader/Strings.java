// Blockheads Club mod loader: UI texts in en, tr, ru, kk. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import java.util.HashMap;
import java.util.Locale;
import java.util.Map;

/** UI texts. English by default; Turkish uses the informal "sen". `{name}` placeholders are filled by {@link #t}. */
final class Strings {
    private static final Map<String, String[]> M = new HashMap<>();
    private static int lang = -1;

    private Strings() {}

    private static void p(String key, String en, String tr, String ru, String kk) {
        M.put(key, new String[] {en, tr, ru, kk});
    }

    static {
        p("mods", "Mods", "Modlar", "Моды", "Модтар");
        p("mark", "Mods {v}", "Modlar {v}", "Моды {v}", "Модтар {v}");
        p("back", "Back", "Geri", "Назад", "Артқа");
        p("cancel", "Cancel", "İptal", "Отмена", "Бас тарту");
        p("close", "Close", "Kapat", "Закрыть", "Жабу");
        p("done", "Done", "Tamam", "Готово", "Дайын");
        p("try_again", "Try again", "Tekrar dene", "Повторить", "Қайталау");
        p("copy_details", "Copy details", "Ayrıntıları kopyala", "Скопировать детали", "Мәліметтерді көшіру");
        p("copied", "Copied", "Kopyalandı", "Скопировано", "Көшірілді");
        p("install", "Install", "Yükle", "Установить", "Орнату");
        p("remove", "Remove", "Kaldır", "Удалить", "Жою");
        p("keep", "Keep", "Kalsın", "Оставить", "Қалдыру");
        p("on", "On", "Açık", "Вкл.", "Қосулы");
        p("off", "Off", "Kapalı", "Выкл.", "Өшірулі");
        p("native", "Native", "Yerel kod", "Нативный", "Нативті");
        p("reviewed", "Reviewed", "İncelendi", "Проверен", "Тексерілген");
        p("not_reviewed", "Not reviewed", "İncelenmedi", "Не проверен", "Тексерілмеген");
        p("side_client", "Client", "İstemci", "Клиент", "Клиент");
        p("side_server", "Server", "Sunucu", "Сервер", "Сервер");
        p("side_both", "Client + server", "İstemci + sunucu", "Клиент + сервер", "Клиент + сервер");
        p("st_running", "Running", "Çalışıyor", "Работает", "Жұмыс істеп тұр");
        p("st_off", "Turned off", "Kapalı", "Выключен", "Өшірулі");
        p("st_next", "Starts when you open a world", "Bir dünya açınca başlar", "Запустится при открытии мира", "Әлем ашылғанда іске қосылады");
        p("by", "by {a}", "yapan: {a}", "автор: {a}", "авторы: {a}");

        // permissions
        p("perm_none", "Nothing extra", "Ek bir şey istemiyor", "Ничего лишнего", "Қосымша ештеңе");
        p("perm_world_read", "Reads the world", "Dünyayı okur", "Читает мир", "Әлемді оқиды");
        p("perm_world_write", "Changes the world", "Dünyayı değiştirir", "Изменяет мир", "Әлемді өзгертеді");
        p("perm_chat", "Chat", "Sohbet", "Чат", "Чат");
        p("perm_players", "Manages players", "Oyuncuları yönetir", "Управляет игроками", "Ойыншыларды басқарады");
        p("perm_channels", "Talks to the server", "Sunucuyla konuşur", "Общается с сервером", "Сервермен байланысады");
        p("perm_net", "Uses the internet", "İnterneti kullanır", "Использует интернет", "Интернетті пайдаланады");
        p("perm_native", "Native · full access", "Yerel kod · tam erişim", "Нативный · полный доступ", "Нативті · толық қатынас");
        p("what_asks", "What it asks for", "Ne istiyor", "Что ему нужно", "Не сұрайды");
        p("danger_title", "Danger · native code", "Tehlike · yerel kod", "Опасно · нативный код", "Қауіпті · нативті код");
        p("danger_body", "{mod} runs outside the sandbox. It can change anything in the game and on this device.",
            "{mod} korumalı alanın dışında çalışır. Oyunda ve bu cihazda her şeyi değiştirebilir.",
            "{mod} работает вне песочницы. Он может изменить что угодно в игре и на этом устройстве.",
            "{mod} құмсалғыштан тыс жұмыс істейді. Ойындағы және осы құрылғыдағы кез келген нәрсені өзгерте алады.");

        p("install_native", "I understand, install", "Anladım, yükle", "Понимаю, установить", "Түсіндім, орнату");
        p("danger_note", "Install it only if you trust the author. You can turn it off or remove it at any time.",
            "Yalnızca yazarına güveniyorsan yükle. İstediğin zaman kapatabilir ya da kaldırabilirsin.",
            "Устанавливай, только если доверяешь автору. Мод можно выключить или удалить в любой момент.",
            "Автордың адалдығына сенсеңіз ғана орнатыңыз. Модты кез келген уақытта өшіруге немесе жоюға болады.");

        // join
        p("join_title_one", "{server} uses 1 mod", "{server} 1 mod kullanıyor", "{server} использует 1 мод", "{server} 1 мод пайдаланады");
        p("join_title", "{server} uses {n} mods", "{server} {n} mod kullanıyor", "{server} использует модов: {n}", "{server} {n} мод пайдаланады");
        p("join_body", "{server} needs these mods. They are downloaded from the server and used only on {server}.",
            "{server} bu modlara ihtiyaç duyuyor. Sunucudan indirilir ve yalnızca {server} üzerinde kullanılır.",
            "{server} нужны эти моды. Они скачиваются с сервера и используются только на {server}.",
            "{server} үшін осы модтар керек. Олар серверден жүктеледі және тек {server} серверінде қолданылады.");
        p("join_optional", "{server} offers these mods. You can join without them.",
            "{server} bu modları sunuyor. Onlarsız da katılabilirsin.",
            "{server} предлагает эти моды. Можно войти и без них.",
            "{server} осы модтарды ұсынады. Оларсыз да кіре аласың.");
        p("you_have", "You already have {mods}", "{mods} zaten sende var", "У тебя уже есть {mods}", "Сенде {mods} бар");
        p("total", "total {size}", "toplam {size}", "всего {size}", "барлығы {size}");
        p("download_join", "Download and join · {size}", "İndir ve katıl · {size}", "Скачать и войти · {size}", "Жүктеп, кіру · {size}");
        p("join", "Join", "Katıl", "Войти", "Кіру");
        p("join_without", "Join without", "Onlarsız katıl", "Войти без них", "Оларсыз кіру");
        p("dont_join", "Don’t join", "Katılma", "Не входить", "Кірмеу");
        p("joining_title", "Joining {server}", "{server} sunucusuna katılıyorsun", "Вход на {server}", "{server} серверіне кіру");
        p("getting", "Getting mods from {server}… {done} of {total}", "{server} sunucusundan modlar alınıyor… {done} / {total}",
            "Загрузка модов с {server}… {done} из {total}", "{server} серверінен модтар жүктелуде… {done} / {total}");
        p("checked", "Checked", "Kontrol edildi", "Проверен", "Тексерілді");
        p("waiting", "Waiting", "Bekliyor", "Ожидание", "Күтуде");
        p("responsive", "The game stays responsive. You join as soon as all files are checked.",
            "Oyun donmaz. Tüm dosyalar kontrol edilince hemen katılırsın.",
            "Игра не зависает. Ты войдёшь, как только все файлы будут проверены.",
            "Ойын қатып қалмайды. Барлық файл тексерілгенде бірден кіресің.");
        p("dl_failed", "Download failed", "İndirme başarısız", "Загрузка не удалась", "Жүктеу сәтсіз");
        p("didnt_download", "{mod} didn’t download", "{mod} indirilemedi", "{mod} не скачался", "{mod} жүктелмеді");
        p("dl_failed_body", "The server could not send the file. Files that finished are saved; they won’t download again.",
            "Sunucu dosyayı gönderemedi. Biten dosyalar kaydedildi; tekrar indirilmeyecek.",
            "Сервер не смог отправить файл. Готовые файлы сохранены и не будут скачиваться снова.",
            "Сервер файлды жібере алмады. Аяқталған файлдар сақталды, қайта жүктелмейді.");
        p("check_failed", "File check failed", "Dosya kontrolü başarısız", "Проверка файла не пройдена", "Файл тексерісі өтпеді");
        p("didnt_pass", "{mod} didn’t pass the file check", "{mod} dosya kontrolünü geçemedi", "{mod} не прошёл проверку файла", "{mod} файл тексерісінен өтпеді");
        p("hash_body", "The file from the server doesn’t match the one the server announced. Nothing was used. Tell the owner of {server}.",
            "Sunucudan gelen dosya, sunucunun duyurduğu dosyayla aynı değil. Hiçbir şey kullanılmadı. {server} sahibine haber ver.",
            "Файл с сервера не совпадает с заявленным. Ничего не использовано. Сообщи владельцу {server}.",
            "Серверден келген файл жарияланған файлмен сәйкес емес. Ештеңе қолданылмады. {server} иесіне хабарла.");
        p("refused_body", "This mod can’t be used: {why}", "Bu mod kullanılamıyor: {why}", "Этот мод нельзя использовать: {why}", "Бұл модты қолдануға болмайды: {why}");
        p("update_title", "Update the mod loader", "Mod yükleyiciyi güncelle", "Обнови загрузчик модов", "Мод жүктегішті жаңарт");
        p("needs_loader", "{server} needs mod loader {v}", "{server} için mod yükleyici {v} gerekiyor", "{server} нужен загрузчик модов {v}", "{server} үшін мод жүктегіш {v} керек");
        p("you_have_loader", "You have {v}.", "Sende {v} var.", "У тебя {v}.", "Сенде {v}.");
        p("update_body", "Open Blockheads Patcher and patch your game again. Your worlds stay.",
            "Blockheads Patcher’ı aç ve oyununu yeniden yamala. Dünyaların kalır.",
            "Открой Blockheads Patcher и пропатчь игру заново. Твои миры сохранятся.",
            "Blockheads Patcher-ді ашып, ойынды қайта патчта. Әлемдерің сақталады.");
        p("back_menu", "Back to menu", "Menüye dön", "В меню", "Мәзірге оралу");
        p("how_update", "How to update", "Nasıl güncellenir", "Как обновить", "Қалай жаңарту керек");
        p("cant_join", "Can’t join {server}", "{server} sunucusuna katılamazsın", "Не удаётся войти на {server}", "{server} серверіне кіру мүмкін емес");
        p("declined", "You didn’t accept the mods {server} needs.", "{server} için gereken modları kabul etmedin.",
            "Ты не принял моды, нужные {server}.", "{server} қажет ететін модтарды қабылдамадың.");
        p("load_failed", "The mods didn’t start.", "Modlar başlamadı.", "Моды не запустились.", "Модтар іске қосылмады.");
        p("stalled", "The download stopped.", "İndirme durdu.", "Загрузка остановилась.", "Жүктеу тоқтап қалды.");
        p("protocol", "The server sent something unexpected.", "Sunucu beklenmedik bir şey gönderdi.", "Сервер прислал что-то неожиданное.", "Сервер күтпеген нәрсе жіберді.");

        // Mods screen
        p("tab_installed", "Installed", "Yüklü", "Установленные", "Орнатылған");
        p("tab_list", "Mod list", "Mod listesi", "Список модов", "Модтар тізімі");
        p("tab_link", "From link", "Bağlantıdan", "По ссылке", "Сілтемеден");
        p("no_mods", "No mods yet", "Henüz mod yok", "Модов пока нет", "Әзірге мод жоқ");
        p("no_mods_body", "Mods add maps, sounds, looks and new things to do. Start with the reviewed mod list.",
            "Modlar harita, ses, görünüm ve yapacak yeni şeyler ekler. İncelenmiş mod listesiyle başla.",
            "Моды добавляют карты, звуки, внешний вид и новые занятия. Начни с проверенного списка.",
            "Модтар карта, дыбыс, көрініс және жаңа істер қосады. Тексерілген тізімнен баста.");
        p("browse", "Browse the mod list", "Mod listesine göz at", "Открыть список модов", "Модтар тізімін ашу");
        p("add_link", "Add from link", "Bağlantıdan ekle", "Добавить по ссылке", "Сілтемеден қосу");
        p("reading", "Reading your mods…", "Modların okunuyor…", "Читаю твои моды…", "Модтарың оқылуда…");
        p("cant_read", "Can’t read your mods", "Modların okunamıyor", "Не удаётся прочитать моды", "Модтарыңды оқу мүмкін емес");
        p("apply_note", "Changes apply when you open a world.", "Değişiklikler bir dünya açınca uygulanır.",
            "Изменения применятся при открытии мира.", "Өзгерістер әлем ашылғанда қолданылады.");
        p("online_note", "Online game — it keeps running while menus are open.", "Çevrimiçi oyun — menüler açıkken de devam eder.",
            "Онлайн-игра продолжается, пока открыто меню.", "Онлайн ойын мәзір ашық тұрғанда да жалғасады.");
        p("remove_q", "Remove {mod}?", "{mod} kaldırılsın mı?", "Удалить {mod}?", "{mod} жойылсын ба?");
        p("remove_body", "Its settings are removed too. You can install it again from the mod list.",
            "Ayarları da silinir. Mod listesinden tekrar yükleyebilirsin.",
            "Его настройки тоже удалятся. Его можно установить снова из списка.",
            "Баптаулары да жойылады. Тізімнен қайта орната аласың.");
        p("search", "Search mods", "Mod ara", "Поиск модов", "Мод іздеу");
        p("loading_list", "Loading the mod list…", "Mod listesi yükleniyor…", "Загрузка списка модов…", "Модтар тізімі жүктелуде…");
        p("offline", "You’re offline", "Çevrimdışısın", "Нет подключения", "Желіде жоқсың");
        p("offline_body", "The mod list needs the internet. Your installed mods still work.",
            "Mod listesi için internet gerekir. Yüklü modların çalışmaya devam eder.",
            "Списку модов нужен интернет. Установленные моды работают.",
            "Модтар тізіміне интернет керек. Орнатылған модтар жұмыс істей береді.");
        p("list_error", "The mod list didn’t load", "Mod listesi yüklenemedi", "Список модов не загрузился", "Модтар тізімі жүктелмеді");
        p("list_error_body", "The list server answered with an error. Try again in a minute.",
            "Liste sunucusu hata verdi. Bir dakika sonra tekrar dene.",
            "Сервер списка ответил ошибкой. Попробуй через минуту.",
            "Тізім сервері қате қайтарды. Бір минуттан соң қайталап көр.");
        p("no_results", "No mods match “{q}”", "“{q}” ile eşleşen mod yok", "Нет модов по запросу «{q}»", "«{q}» бойынша мод жоқ");
        p("clear_search", "Clear search", "Aramayı temizle", "Очистить поиск", "Іздеуді тазарту");
        p("install_q", "Install {mod}?", "{mod} yüklensin mi?", "Установить {mod}?", "{mod} орнатылсын ба?");
        p("installing", "Installing {mod}", "{mod} yükleniyor", "Установка {mod}", "{mod} орнатылуда");
        p("downloading", "Downloading {mod}… {done} of {total}", "{mod} indiriliyor… {done} / {total}",
            "Загрузка {mod}… {done} из {total}", "{mod} жүктелуде… {done} / {total}");
        p("bg_note", "You can close this. It keeps going in the background and won’t slow the game.",
            "Bunu kapatabilirsin. Arka planda devam eder ve oyunu yavaşlatmaz.",
            "Это окно можно закрыть. Загрузка продолжится в фоне и не замедлит игру.",
            "Мұны жаба аласың. Фонда жалғасады және ойынды баяулатпайды.");
        p("hide", "Hide", "Gizle", "Скрыть", "Жасыру");
        p("installed_title", "Installed", "Yüklendi", "Установлено", "Орнатылды");
        p("installed_body", "{mod} is installed. It starts when you open a world.",
            "{mod} yüklendi. Bir dünya açınca başlar.", "{mod} установлен. Он запустится при открытии мира.",
            "{mod} орнатылды. Әлем ашылғанда іске қосылады.");
        p("install_failed", "Install failed", "Yükleme başarısız", "Установка не удалась", "Орнату сәтсіз");
        p("link_note", "Mods from links and files are not reviewed", "Bağlantı ve dosyalardan gelen modlar incelenmez",
            "Моды по ссылкам и из файлов не проверяются", "Сілтеме мен файлдан келген модтар тексерілмейді");
        p("link_note2", "You always see what they ask for before anything is installed.",
            "Bir şey yüklenmeden önce ne istediklerini her zaman görürsün.",
            "Перед установкой ты всегда видишь, что им нужно.",
            "Орнатпас бұрын олардың не сұрайтынын әрқашан көресің.");
        p("paste", "Paste", "Yapıştır", "Вставить", "Қою");
        p("check_link", "Check link", "Bağlantıyı kontrol et", "Проверить ссылку", "Сілтемені тексеру");
        p("checking", "Checking…", "Kontrol ediliyor…", "Проверка…", "Тексерілуде…");
        p("link_q", "Install from a link?", "Bağlantıdan yüklensin mi?", "Установить по ссылке?", "Сілтемеден орнату керек пе?");
        p("link_warn", "This mod is not from the mod list", "Bu mod, mod listesinden değil", "Этого мода нет в списке", "Бұл мод тізімнен емес");
        p("link_warn2", "No one has checked it. Install it only if you trust where it came from.",
            "Kimse kontrol etmedi. Yalnızca geldiği yere güveniyorsan yükle.",
            "Его никто не проверял. Устанавливай, только если доверяешь источнику.",
            "Оны ешкім тексермеген. Көзіне сенсең ғана орнат.");
        p("install_anyway", "Install anyway", "Yine de yükle", "Всё равно установить", "Бәрібір орнату");
        p("bad_link", "That link didn’t give a mod file.", "Bu bağlantıdan bir mod dosyası gelmedi.",
            "По этой ссылке нет файла мода.", "Бұл сілтемеден мод файлы алынбады.");
        p("from_link", "from a link", "bir bağlantıdan", "по ссылке", "сілтемеден");
        p("server_only_note", "Only for servers: install it on a server, not in the game.",
            "Yalnızca sunucular için: oyuna değil, bir sunucuya yükle.",
            "Только для серверов: ставится на сервер, а не в игру.",
            "Тек серверлер үшін: ойынға емес, серверге орнатылады.");

        // welcome
        p("w1_title", "Mods are ready", "Modlar hazır", "Моды готовы", "Модтар дайын");
        p("w1_body", "Your game now has the Blockheads Club mod loader {v}. Mods add maps, sounds, looks and new things to do.",
            "Oyununda artık Blockheads Club mod yükleyici {v} var. Modlar harita, ses, görünüm ve yapacak yeni şeyler ekler.",
            "В твоей игре теперь есть загрузчик модов Blockheads Club {v}. Моды добавляют карты, звуки, внешний вид и новые занятия.",
            "Ойыныңда енді Blockheads Club мод жүктегіші {v} бар. Модтар карта, дыбыс, көрініс және жаңа істер қосады.");
        p("w1_body2", "Nothing is installed yet. Your worlds are not changed.", "Henüz hiçbir şey yüklenmedi. Dünyaların değişmedi.",
            "Пока ничего не установлено. Твои миры не изменены.", "Әзірге ештеңе орнатылмаған. Әлемдерің өзгермеген.");
        p("w2_title", "Find them in Mods", "Onları Modlar’da bul", "Ищи их в разделе «Моды»", "Оларды «Модтар» бөлімінен тап");
        p("w2_l1", "Tap the Mods mark on the title screen.", "Başlık ekranındaki Modlar işaretine dokun.",
            "Нажми на значок «Моды» на титульном экране.", "Басты экрандағы «Модтар» белгісін бас.");
        p("w2_l2", "Mod list: mods checked by Blockheads Club maintainers.", "Mod listesi: Blockheads Club bakımcılarının kontrol ettiği modlar.",
            "Список модов: моды, проверенные командой Blockheads Club.", "Модтар тізімі: Blockheads Club жүргізушілері тексерген модтар.");
        p("w2_l3", "Servers can use mods. You see them and agree before anything downloads.",
            "Sunucular mod kullanabilir. Bir şey inmeden önce onları görür ve onaylarsın.",
            "Серверы могут использовать моды. Ты увидишь их и согласишься до загрузки.",
            "Серверлер мод қолдана алады. Жүктелмес бұрын оларды көріп, келісесің.");
        p("w3_title", "Stay safe", "Güvende kal", "Будь осторожен", "Қауіпсіз бол");
        p("w3_reviewed", "On the mod list. A maintainer checked it.", "Mod listesinde. Bir bakımcı kontrol etti.",
            "Есть в списке. Проверен командой.", "Тізімде бар. Жүргізуші тексерген.");
        p("w3_not", "From a link or file. No one checked it.", "Bir bağlantıdan ya da dosyadan. Kimse kontrol etmedi.",
            "По ссылке или из файла. Никто не проверял.", "Сілтемеден не файлдан. Ешкім тексермеген.");
        p("w3_native", "Not sandboxed. It can change anything. You always confirm it first.",
            "Korumalı alanda değil. Her şeyi değiştirebilir. Her zaman önce sen onaylarsın.",
            "Без песочницы. Может изменить что угодно. Ты всегда подтверждаешь это заранее.",
            "Құмсалғышсыз. Кез келген нәрсені өзгерте алады. Әрқашан алдымен сен растайсың.");
        p("w3_note", "Most mods run in a sandbox with time and memory limits. A mod that keeps slowing the game is turned off by itself.",
            "Çoğu mod zaman ve bellek sınırları olan korumalı bir alanda çalışır. Oyunu sürekli yavaşlatan bir mod kendiliğinden kapanır.",
            "Большинство модов работают в песочнице с ограничениями времени и памяти. Мод, который тормозит игру, отключается сам.",
            "Көп мод уақыт пен жад шектеулері бар құмсалғышта жұмыс істейді. Ойынды үнемі баяулататын мод өздігінен өшеді.");
        p("skip", "Skip", "Geç", "Пропустить", "Өткізу");
        p("next", "Next", "İleri", "Далее", "Келесі");
        p("play", "Play", "Oyna", "Играть", "Ойнау");
        p("open_mods", "Open Mods", "Modlar’ı aç", "Открыть моды", "Модтарды ашу");
    }

    static int lang() {
        if (lang < 0) {
            String l = Locale.getDefault().getLanguage();
            lang = "tr".equals(l) ? 1 : "ru".equals(l) ? 2 : "kk".equals(l) ? 3 : 0;
        }
        return lang;
    }

    /** The text for `key` with `{name}` placeholders replaced from `kv` pairs. Missing keys return the key. */
    static String t(String key, Object... kv) {
        String[] v = M.get(key);
        String s = v == null ? key : v[lang()];
        for (int i = 0; i + 1 < kv.length; i += 2) {
            s = s.replace("{" + kv[i] + "}", String.valueOf(kv[i + 1]));
        }
        return s;
    }

    /** A localized name from a `{lang: text}` map (JSON), falling back to English, then any value, then `fallback`. */
    static String pick(org.json.JSONObject names, String fallback) {
        if (names == null) return fallback;
        String[] codes = {"en", "tr", "ru", "kk"};
        String s = names.optString(codes[lang()], "");
        if (s.isEmpty()) s = names.optString("en", "");
        if (s.isEmpty() && names.keys().hasNext()) s = names.optString(names.keys().next(), "");
        return s.isEmpty() ? fallback : s;
    }
}
