//! The Apple Watch strings (Wristper fork): the Apple Watch pane and its
//! first-run page, in every interface language. They live apart from
//! [`super::TRANSLATIONS`] so that table stays under its size cap; each
//! language's table takes its column from here ([`column`]), and
//! Traditional Chinese is converted from the Simplified one as everywhere
//! else. Columns: Polish, Chinese (Simplified), Japanese, German, Spanish,
//! Ukrainian, Brazilian Portuguese, French, Russian. Not reviewed by native
//! speakers.

/// Columns of [`TRANSLATIONS`] in the order of [`super::column_for_code`]'s
/// first six languages, then Portuguese, French, and Russian.
pub(super) const LANGUAGES: usize = 9;
const PORTUGUESE: usize = 6;
const FRENCH: usize = 7;
const RUSSIAN: usize = 8;

/// This file's strings for `column` of the main table (0 through 5).
pub(super) fn column(column: usize) -> impl Iterator<Item = (&'static str, &'static str)> {
    TRANSLATIONS
        .iter()
        .map(move |(english, translated)| (*english, translated[column]))
}

pub(super) fn portuguese() -> impl Iterator<Item = (&'static str, &'static str)> {
    column(PORTUGUESE)
}

pub(super) fn french() -> impl Iterator<Item = (&'static str, &'static str)> {
    column(FRENCH)
}

pub(super) fn russian() -> impl Iterator<Item = (&'static str, &'static str)> {
    column(RUSSIAN)
}

/// The Simplified Chinese column, which Traditional Chinese converts.
pub(super) fn simplified() -> impl Iterator<Item = (&'static str, &'static str)> {
    column(1)
}

#[rustfmt::skip]
pub(super) const TRANSLATIONS: &[(&str, [&str; LANGUAGES])] = &[
    ("Apple Watch", ["Apple Watch", "Apple Watch", "Apple Watch", "Apple Watch", "Apple Watch", "Apple Watch", "Apple Watch", "Apple Watch", "Apple Watch"]),
    ("This Mac", ["Ten Mac", "这台 Mac", "この Mac", "Dieser Mac", "Este Mac", "Цей Mac", "Este Mac", "Ce Mac", "Этот Mac"]),
    ("Connection", ["Połączenie", "连接", "接続", "Verbindung", "Conexión", "З'єднання", "Conexão", "Connexion", "Соединение"]),
    ("Connected", ["Połączono", "已连接", "接続済み", "Verbunden", "Conectado", "З'єднано", "Conectado", "Connecté", "Подключено"]),
    ("Offline: check the network", ["Offline: sprawdź sieć", "离线：请检查网络", "オフライン: ネットワークを確認してください", "Offline: Netzwerk prüfen", "Sin conexión: revisa la red", "Офлайн: перевірте мережу", "Offline: verifique a rede", "Hors ligne : vérifiez le réseau", "Нет связи: проверьте сеть"]),
    ("Another Mac took over this pairing", ["Inny Mac przejął to parowanie", "另一台 Mac 接管了此配对", "別の Mac がこのペアリングを引き継ぎました", "Ein anderer Mac hat diese Kopplung übernommen", "Otro Mac se ha hecho cargo de este emparejamiento", "Інший Mac перебрав це спарювання", "Outro Mac assumiu este pareamento", "Un autre Mac a repris ce jumelage", "Другой Mac перехватил это сопряжение"]),
    ("Name on your watch", ["Nazwa na zegarku", "手表上显示的名称", "ウォッチに表示される名前", "Name auf deiner Uhr", "Nombre en tu reloj", "Назва на годиннику", "Nome no seu relógio", "Nom sur votre montre", "Имя на часах"]),
    ("What the watch shows for this Mac", ["To, co zegarek pokazuje dla tego Maca", "手表上显示的这台 Mac 的名称", "ウォッチがこの Mac に付ける名前", "So zeigt die Uhr diesen Mac an", "Cómo muestra el reloj este Mac", "Як годинник показує цей Mac", "Como o relógio mostra este Mac", "Comment la montre affiche ce Mac", "Как часы показывают этот Mac"]),
    ("Pair a watch", ["Sparuj zegarek", "配对手表", "ウォッチをペアリング", "Uhr koppeln", "Emparejar un reloj", "Спарувати годинник", "Parear um relógio", "Jumeler une montre", "Сопрячь часы"]),
    ("Pairing code", ["Kod parowania", "配对码", "ペアリングコード", "Kopplungscode", "Código de emparejamiento", "Код спарювання", "Código de pareamento", "Code de jumelage", "Код сопряжения"]),
    ("On your watch, open Wristper › Pair a Mac and type the code it shows", ["Na zegarku otwórz Wristper › Sparuj Maca i wpisz wyświetlony kod", "在手表上打开 Wristper › 配对 Mac，然后输入显示的代码", "ウォッチで Wristper › Mac をペアリング を開き、表示されるコードを入力します", "Öffne auf der Uhr Wristper › Mac koppeln und gib den angezeigten Code ein", "En el reloj, abre Wristper › Emparejar un Mac y escribe el código que aparece", "На годиннику відкрийте Wristper › Спарувати Mac і введіть код, який він покаже", "No relógio, abra Wristper › Parear um Mac e digite o código exibido", "Sur la montre, ouvrez Wristper › Jumeler un Mac et saisissez le code affiché", "На часах откройте Wristper › Сопрячь Mac и введите показанный код"]),
    ("Get the watch app", ["Pobierz aplikację na zegarek", "获取手表 App", "ウォッチ App を入手", "Watch-App laden", "Obtener la app del reloj", "Отримати застосунок для годинника", "Obter o app do relógio", "Obtenir l'app pour la montre", "Скачать приложение для часов"]),
    ("Paired watches", ["Sparowane zegarki", "已配对的手表", "ペアリング済みのウォッチ", "Gekoppelte Uhren", "Relojes emparejados", "Спаровані годинники", "Relógios pareados", "Montres jumelées", "Сопряжённые часы"]),
    ("Remove…", ["Usuń…", "移除…", "削除…", "Entfernen …", "Quitar…", "Вилучити…", "Remover…", "Supprimer…", "Удалить…"]),
    ("No watches paired yet", ["Nie sparowano jeszcze żadnego zegarka", "还没有配对的手表", "ペアリング済みのウォッチはまだありません", "Noch keine Uhr gekoppelt", "Aún no hay relojes emparejados", "Годинників ще не спаровано", "Nenhum relógio pareado ainda", "Aucune montre jumelée pour l'instant", "Пока нет сопряжённых часов"]),
    ("Couldn't remove {name}: {error}", ["Nie udało się usunąć {name}: {error}", "无法移除 {name}：{error}", "{name} を削除できませんでした: {error}", "{name} konnte nicht entfernt werden: {error}", "No se pudo quitar {name}: {error}", "Не вдалося вилучити {name}: {error}", "Não foi possível remover {name}: {error}", "Impossible de supprimer {name} : {error}", "Не удалось удалить {name}: {error}"]),
    ("Last seen {}", ["Ostatnio widziano {}", "最后在线：{}", "最終確認: {}", "Zuletzt gesehen {}", "Visto por última vez {}", "Востаннє на зв'язку {}", "Visto pela última vez {}", "Vu pour la dernière fois {}", "Последний раз на связи {}"]),
    ("Pair it again to use it", ["Sparuj go ponownie, aby go używać", "请重新配对后再使用", "使用するには再度ペアリングしてください", "Zum Verwenden erneut koppeln", "Vuelve a emparejarlo para usarlo", "Спаруйте його знову, щоб користуватися", "Pareie novamente para usar", "Jumelez-la à nouveau pour l'utiliser", "Сопрягите снова, чтобы пользоваться"]),
    ("Waiting for the keychain", ["Czekam na pęk kluczy", "正在等待钥匙串", "キーチェーンを待っています", "Warte auf den Schlüsselbund", "Esperando al llavero", "Очікування на в'язку ключів", "Aguardando o chaveiro", "En attente du trousseau", "Ожидание связки ключей"]),
    ("Status and paste", ["Stan i wklejanie", "状态与粘贴", "ステータスと貼り付け", "Status und Einfügen", "Estado y pegado", "Стан і вставлення", "Status e colagem", "État et collage", "Статус и вставка"]),
    ("Status capsule", ["Kapsuła stanu", "状态胶囊", "ステータスカプセル", "Status-Kapsel", "Cápsula de estado", "Капсула стану", "Cápsula de status", "Capsule d'état", "Капсула статуса"]),
    ("Shows what a watch dictation is doing, near the bottom of the screen", ["Pokazuje u dołu ekranu, co dzieje się z dyktowaniem z zegarka", "在屏幕底部附近显示手表听写的进度", "画面の下部に、ウォッチからの音声入力の状況を表示します", "Zeigt unten am Bildschirm, was mit einem Uhren-Diktat passiert", "Muestra, cerca de la parte inferior de la pantalla, qué hace un dictado del reloj", "Показує внизу екрана, що відбувається з диктуванням із годинника", "Mostra, perto da parte inferior da tela, o que um ditado do relógio está fazendo", "Affiche, en bas de l'écran, ce que fait une dictée de la montre", "Показывает у нижнего края экрана, что происходит с диктовкой с часов"]),
    ("Follow the pointer", ["Podążaj za wskaźnikiem", "跟随指针", "ポインタに追従", "Dem Zeiger folgen", "Seguir el puntero", "Слідувати за вказівником", "Seguir o ponteiro", "Suivre le pointeur", "Следовать за указателем"]),
    ("Shows the capsule on the screen the pointer is on", ["Pokazuje kapsułę na ekranie, na którym jest wskaźnik", "在指针所在的屏幕上显示胶囊", "ポインタがある画面にカプセルを表示します", "Zeigt die Kapsel auf dem Bildschirm, auf dem der Zeiger ist", "Muestra la cápsula en la pantalla donde está el puntero", "Показує капсулу на екрані, де перебуває вказівник", "Mostra a cápsula na tela em que o ponteiro está", "Affiche la capsule sur l'écran où se trouve le pointeur", "Показывает капсулу на экране, где находится указатель"]),
    ("A watch dictation stays on the clipboard after it pastes", ["Dyktowanie z zegarka zostaje w schowku po wklejeniu", "手表听写的内容在粘贴后会保留在剪贴板", "ウォッチからの音声入力は、貼り付け後もクリップボードに残ります", "Ein Uhren-Diktat bleibt nach dem Einfügen in der Zwischenablage", "Un dictado del reloj se queda en el portapapeles después de pegarse", "Диктування з годинника залишається в буфері обміну після вставлення", "Um ditado do relógio fica na área de transferência depois de colado", "Une dictée de la montre reste dans le presse-papiers après le collage", "Диктовка с часов остаётся в буфере обмена после вставки"]),
    ("The Apple Watch service isn't running", ["Usługa Apple Watch nie działa", "Apple Watch 服务未运行", "Apple Watch サービスが実行されていません", "Der Apple-Watch-Dienst läuft nicht", "El servicio de Apple Watch no se está ejecutando", "Служба Apple Watch не працює", "O serviço do Apple Watch não está em execução", "Le service Apple Watch ne tourne pas", "Служба Apple Watch не запущена"]),
    ("Type the six digits your watch shows", ["Wpisz sześć cyfr z zegarka", "请输入手表上显示的六位数字", "ウォッチに表示される 6 桁の数字を入力してください", "Gib die sechs Ziffern ein, die deine Uhr zeigt", "Escribe los seis dígitos que muestra el reloj", "Введіть шість цифр, які показує годинник", "Digite os seis dígitos exibidos no relógio", "Saisissez les six chiffres affichés par la montre", "Введите шесть цифр, показанных на часах"]),
    ("Hex can't reach the relay. Check the network and try again", ["Hex nie może połączyć się z przekaźnikiem. Sprawdź sieć i spróbuj ponownie", "Hex 无法连接到中继服务器。请检查网络后重试", "Hex がリレーに接続できません。ネットワークを確認してもう一度お試しください", "Hex erreicht das Relay nicht. Prüfe das Netzwerk und versuche es erneut", "Hex no puede conectar con el relay. Revisa la red e inténtalo de nuevo", "Hex не може під'єднатися до ретранслятора. Перевірте мережу й спробуйте ще раз", "O Hex não consegue acessar o relay. Verifique a rede e tente de novo", "Hex ne peut pas joindre le relais. Vérifiez le réseau et réessayez", "Hex не может связаться с ретранслятором. Проверьте сеть и повторите попытку"]),
    ("The relay didn't answer. Try again", ["Przekaźnik nie odpowiedział. Spróbuj ponownie", "中继服务器没有响应。请重试", "リレーが応答しませんでした。もう一度お試しください", "Das Relay hat nicht geantwortet. Versuche es erneut", "El relay no respondió. Inténtalo de nuevo", "Ретранслятор не відповів. Спробуйте ще раз", "O relay não respondeu. Tente de novo", "Le relais n'a pas répondu. Réessayez", "Ретранслятор не ответил. Повторите попытку"]),
    ("No watch is showing that code. It may have expired", ["Żaden zegarek nie pokazuje tego kodu. Mógł wygasnąć", "没有手表显示该代码，它可能已过期", "そのコードを表示しているウォッチがありません。期限切れの可能性があります", "Keine Uhr zeigt diesen Code. Er ist vielleicht abgelaufen", "Ningún reloj muestra ese código. Puede que haya caducado", "Жоден годинник не показує цей код. Можливо, він застарів", "Nenhum relógio está exibindo esse código. Ele pode ter expirado", "Aucune montre n'affiche ce code. Il a peut-être expiré", "Ни одни часы не показывают этот код. Возможно, он устарел"]),
    ("Too many wrong codes. Wait 10 minutes", ["Zbyt wiele błędnych kodów. Poczekaj 10 minut", "错误代码次数过多，请等待 10 分钟", "コードの誤りが多すぎます。10 分待ってください", "Zu viele falsche Codes. Warte 10 Minuten", "Demasiados códigos incorrectos. Espera 10 minutos", "Забагато неправильних кодів. Зачекайте 10 хвилин", "Muitos códigos incorretos. Aguarde 10 minutos", "Trop de codes erronés. Attendez 10 minutes", "Слишком много неверных кодов. Подождите 10 минут"]),
    ("The relay refused the code ({})", ["Przekaźnik odrzucił kod ({})", "中继服务器拒绝了该代码（{}）", "リレーがコードを拒否しました（{}）", "Das Relay hat den Code abgelehnt ({})", "El relay rechazó el código ({})", "Ретранслятор відхилив код ({})", "O relay recusou o código ({})", "Le relais a refusé le code ({})", "Ретранслятор отклонил код ({})"]),
    ("Pairing…", ["Parowanie…", "正在配对…", "ペアリング中…", "Kopplung läuft …", "Emparejando…", "Спарювання…", "Pareando…", "Jumelage…", "Сопряжение…"]),
    ("Finish on your watch…", ["Dokończ na zegarku…", "请在手表上完成…", "ウォッチで完了してください…", "Schließe es auf der Uhr ab …", "Termina en el reloj…", "Завершіть на годиннику…", "Conclua no relógio…", "Terminez sur la montre…", "Завершите на часах…"]),
    ("Paired with {}", ["Sparowano z: {}", "已与 {} 配对", "{} とペアリングしました", "Mit {} gekoppelt", "Emparejado con {}", "Спаровано з: {}", "Pareado com {}", "Jumelé avec {}", "Сопряжено с: {}"]),
    ("The watch didn't finish pairing. Update Wristper on the watch and try again", ["Zegarek nie dokończył parowania. Zaktualizuj Wristper na zegarku i spróbuj ponownie", "手表没有完成配对。请更新手表上的 Wristper 后重试", "ウォッチがペアリングを完了しませんでした。ウォッチの Wristper を更新してもう一度お試しください", "Die Uhr hat die Kopplung nicht abgeschlossen. Aktualisiere Wristper auf der Uhr und versuche es erneut", "El reloj no terminó el emparejamiento. Actualiza Wristper en el reloj e inténtalo de nuevo", "Годинник не завершив спарювання. Оновіть Wristper на годиннику й спробуйте ще раз", "O relógio não concluiu o pareamento. Atualize o Wristper no relógio e tente de novo", "La montre n'a pas terminé le jumelage. Mettez Wristper à jour sur la montre et réessayez", "Часы не завершили сопряжение. Обновите Wristper на часах и повторите попытку"]),
    ("Set up your Apple Watch", ["Skonfiguruj Apple Watch", "设置你的 Apple Watch", "Apple Watch を設定", "Apple Watch einrichten", "Configura tu Apple Watch", "Налаштуйте Apple Watch", "Configure seu Apple Watch", "Configurez votre Apple Watch", "Настройте Apple Watch"]),
    ("Dictate from your wrist and Hex pastes the words on this Mac.", ["Dyktuj z nadgarstka, a Hex wklei słowa na tym Macu.", "在手腕上听写，Hex 会把文字粘贴到这台 Mac 上。", "手首から音声入力すると、Hex がこの Mac に文字を貼り付けます。", "Diktiere vom Handgelenk, und Hex fügt den Text auf diesem Mac ein.", "Dicta desde la muñeca y Hex pega las palabras en este Mac.", "Диктуйте із зап'ястя, а Hex вставить слова на цьому Mac.", "Dite do pulso e o Hex cola as palavras neste Mac.", "Dictez depuis votre poignet et Hex colle les mots sur ce Mac.", "Диктуйте с запястья, а Hex вставит слова на этом Mac."]),
    ("Pair later", ["Sparuj później", "稍后配对", "あとでペアリング", "Später koppeln", "Emparejar más tarde", "Спарувати пізніше", "Parear depois", "Jumeler plus tard", "Сопрячь позже"]),
    ("Let Hex type for you", ["Pozwól Hexowi pisać za Ciebie", "允许 Hex 替你输入", "Hex に入力を任せる", "Hex für dich tippen lassen", "Deja que Hex escriba por ti", "Дозвольте Hex друкувати за вас", "Deixe o Hex digitar por você", "Laissez Hex taper pour vous", "Разрешите Hex печатать за вас"]),
    ("Pastes what you dictate on your watch into the app you're using", ["Wkleja to, co podyktujesz na zegarku, do używanej aplikacji", "把你在手表上听写的内容粘贴到你正在使用的 App 中", "ウォッチで音声入力した内容を、使っている App に貼り付けます", "Fügt, was du auf der Uhr diktierst, in die App ein, die du gerade benutzt", "Pega lo que dictas en el reloj en la app que estás usando", "Вставляє те, що ви продиктували на годиннику, в застосунок, яким ви користуєтеся", "Cola o que você dita no relógio no app que você está usando", "Colle ce que vous dictez sur la montre dans l'app que vous utilisez", "Вставляет то, что вы продиктовали на часах, в приложение, которым вы пользуетесь"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_here_repeats_the_main_table_or_itself() {
        let main: std::collections::HashSet<_> = super::super::TRANSLATIONS
            .iter()
            .map(|(english, _)| *english)
            .collect();
        let mut seen = std::collections::HashSet::new();
        for (english, translated) in TRANSLATIONS {
            assert!(!main.contains(english), "{english:?} is in the main table");
            assert!(seen.insert(english), "{english:?} is listed twice");
            assert!(
                translated.iter().all(|text| !text.is_empty()),
                "{english:?}"
            );
            for placeholder in ["{}", "{name}", "{error}"] {
                if english.contains(placeholder) {
                    assert!(
                        translated.iter().all(|text| text.contains(placeholder)),
                        "{english:?} lost {placeholder}"
                    );
                }
            }
        }
    }
}
