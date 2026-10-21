//! UI texts in German, English and Spanish. The language is a process-wide setting.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    De,
    En,
    Es,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::De, Lang::En, Lang::Es];

    pub fn code(self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
            Lang::Es => "es",
        }
    }

    pub fn from_code(s: &str) -> Option<Lang> {
        Self::ALL.into_iter().find(|l| l.code().eq_ignore_ascii_case(s.trim()))
    }
}

pub fn lang() -> Lang {
    Lang::ALL[usize::from(CURRENT.load(Ordering::Relaxed)).min(2)]
}

pub fn set_lang(l: Lang) {
    CURRENT.store(l as u8, Ordering::Relaxed);
}

/// Text for `k` with `{0}`, `{1}`... replaced by `args`.
pub fn tr_with(k: Key, args: &[&dyn Display]) -> String {
    let mut s = tr(k).to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), &a.to_string());
    }
    s
}

/// Server and transport messages arrive in German; translates the known ones.
pub fn server_text(msg: &str) -> String {
    let (head, tail) = msg.split_once(": ").map_or((msg, ""), |(h, t)| (h, t));
    let key = match head {
        "Verbindung fehlgeschlagen" => Key::ConnFailed,
        "Verbindung getrennt" => Key::ConnClosed,
        "Ungültiger Name." => Key::SrvInvalidName,
        "Der Name ist bereits vergeben." => Key::SrvNameTaken,
        "Der Server ist voll." => Key::SrvFull,
        "Falsche Spielversion." => Key::SrvVersion,
        _ => return msg.to_string(),
    };
    if tail.is_empty() {
        tr(key).to_string()
    } else {
        format!("{}: {tail}", tr(key))
    }
}

macro_rules! texts {
    ($($id:ident: $de:expr, $en:expr, $es:expr;)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Key { $($id),* }

        impl Key {
            #[cfg(test)]
            const ALL: &'static [Key] = &[$(Key::$id),*];
        }

        pub fn tr(k: Key) -> &'static str {
            let i = lang() as usize;
            match k { $(Key::$id => [$de, $en, $es][i]),* }
        }
    };
}

texts! {
    GameTitle: "Pinguin Bomberman", "Penguin Bomberman", "Pingüino Bomberman";
    Loading: "Lade Spieldaten...", "Loading game data...", "Cargando datos del juego...";

    HudBombs: "Bomben", "Bombs", "Bombas";
    HudFlame: "Flamme", "Flame", "Llama";
    HudSpeed: "Tempo", "Speed", "Veloc.";
    HudKick: "Kick", "Kick", "Patada";
    Yes: "ja", "yes", "sí";
    SoundOn: "Ton: an", "Sound: on", "Sonido: sí";
    SoundOff: "Ton: aus", "Sound: off", "Sonido: no";
    HintSolo: "WASD / Pfeile + Leertaste / Enter", "WASD / arrows + Space / Enter", "WASD / flechas + Espacio / Enter";
    HintTwo: "1: WASD + Leertaste   2: Pfeile + Enter", "1: WASD + Space   2: Arrows + Enter", "1: WASD + Espacio   2: Flechas + Enter";
    MenuLeave: "Menü (Esc)", "Menu (Esc)", "Menú (Esc)";
    Surrender: "Aufgeben (Esc)", "Surrender (Esc)", "Rendirse (Esc)";

    MenuVsAi: "Gegen Computer", "Vs Computer", "Contra el ordenador";
    MenuTwo: "Zwei Spieler", "Two Players", "Dos jugadores";
    MenuOnline: "Online-Lobby", "Online Lobby", "Sala en línea";
    MenuSettings: "Einstellungen", "Settings", "Ajustes";
    Items: "Items", "Items", "Objetos";
    ItemBomb: "Bombe: eine Bombe mehr", "Bomb: one more bomb", "Bomba: una bomba más";
    ItemSpeed: "Tempo: schneller laufen", "Speed: run faster", "Velocidad: correr más rápido";
    ItemBlast: "Flamme: größere Explosion", "Flame: bigger explosion", "Llama: explosión mayor";
    ItemKick: "Kick: Bomben wegschieben", "Kick: push bombs away", "Patada: empujar bombas";
    PlayerName: "Spielername", "Player name", "Nombre del jugador";
    Controls: "Steuerung", "Controls", "Controles";
    CtrlP1: "Spieler 1", "Player 1", "Jugador 1";
    CtrlP1Move: "  W A S D: laufen", "  W A S D: move", "  W A S D: mover";
    CtrlP1Bomb: "  Leertaste: Bombe", "  Space: bomb", "  Espacio: bomba";
    CtrlP2: "Spieler 2 (Zwei Spieler)", "Player 2 (Two Players)", "Jugador 2 (Dos jugadores)";
    CtrlP2Move: "  Pfeiltasten: laufen", "  Arrow keys: move", "  Flechas: mover";
    CtrlP2Bomb: "  Enter: Bombe", "  Enter: bomb", "  Enter: bomba";
    CtrlAlone: "Allein: beide Tastensätze", "Alone: both key sets", "Solo: ambos juegos de teclas";
    CtrlRule: "Wer in die Flammen läuft, verliert.", "Run into the flames and you lose.", "Quien entre en las llamas, pierde.";
    TitleHint: "Pfeiltasten / Maus: wählen    Enter: starten    1-4: Direktwahl", "Arrow keys / mouse: select    Enter: start    1-4: shortcut", "Flechas / ratón: elegir    Enter: iniciar    1-4: acceso directo";

    Settings: "Einstellungen", "Settings", "Ajustes";
    NameRule: "Bis zu 20 Zeichen: Buchstaben, Ziffern, Leerzeichen, _ - .", "Up to 20 characters: letters, digits, spaces, _ - .", "Hasta 20 caracteres: letras, dígitos, espacios, _ - .";
    NameEmpty: "Bitte einen Namen eingeben.", "Please enter a name.", "Introduce un nombre.";
    Sound: "Ton", "Sound", "Sonido";
    Language: "Sprache", "Language", "Idioma";
    Done: "Fertig", "Done", "Hecho";
    SettingsHint: "Enter: übernehmen    Esc: abbrechen", "Enter: apply    Esc: cancel", "Enter: aplicar    Esc: cancelar";

    Result: "Ergebnis", "Result", "Resultado";
    Draw: "Untenschieden!", "Draw!", "¡Empate!";
    Won: "Gewonnen!!!", "Victory!!!", "¡Victoria!";
    Lost: "Leider verloren...", "Sorry, you lost...", "Has perdido...";
    Player2: "Spieler 2", "Player 2", "Jugador 2";
    Computer: "Computer", "Computer", "Ordenador";
    ComputerN: "Computer {0}", "Computer {0}", "Ordenador {0}";
    WhatNow: "und jetzt? Entscheiden Sie...", "and now? You decide...", "¿y ahora? Decide...";
    AfterHint: "Enter: nochmal spielen    Esc: zurück zum Menü", "Enter: play again    Esc: back to menu", "Enter: jugar otra vez    Esc: volver al menú";
    PlayAgain: "Nochmal spielen", "Play again", "Jugar otra vez";
    ToMenu: "Zum Menü", "To menu", "Al menú";

    OnlineLobby: "Online-Lobby", "Online Lobby", "Sala en línea";
    Name: "Name", "Name", "Nombre";
    ServerAddr: "Server (Adresse:Port)", "Server (address:port)", "Servidor (dirección:puerto)";
    Connect: "Verbinden", "Connect", "Conectar";
    Back: "Zurück", "Back", "Atrás";
    FormHint: "Tab: Feld wechseln    Enter: verbinden    Esc: zurück", "Tab: switch field    Enter: connect    Esc: back", "Tab: cambiar campo    Enter: conectar    Esc: volver";
    InvalidName: "Ungültiger Name (1-20 Zeichen: Buchstaben, Ziffern, Leerzeichen, _ - .)", "Invalid name (1-20 characters: letters, digits, spaces, _ - .)", "Nombre no válido (1-20 caracteres: letras, dígitos, espacios, _ - .)";
    Connecting: "Verbinde...", "Connecting...", "Conectando...";
    ConnectingTo: "Verbinde mit {0} ...", "Connecting to {0} ...", "Conectando con {0} ...";
    Cancel: "Abbrechen", "Cancel", "Cancelar";
    ConnFailed: "Verbindung fehlgeschlagen", "Connection failed", "Conexión fallida";
    ConnClosed: "Verbindung getrennt", "Connection closed", "Conexión cerrada";
    SrvInvalidName: "Ungültiger Name.", "Invalid name.", "Nombre no válido.";
    SrvNameTaken: "Der Name ist bereits vergeben.", "The name is already taken.", "El nombre ya está en uso.";
    SrvFull: "Der Server ist voll.", "The server is full.", "El servidor está lleno.";
    SrvVersion: "Falsche Spielversion.", "Wrong game version.", "Versión del juego incorrecta.";

    Room: "Spielerraum", "Player Room", "Sala de jugadores";
    RoomOn: "{0} auf {1}", "{0} on {1}", "{0} en {1}";
    ColPlayer: "Spieler", "Player", "Jugador";
    ColRecord: "G/V/U", "W/L/D", "G/P/E";
    PlayerCount: "{0} Spieler", "{0} players", "{0} jugadores";
    StatTotal: "Gesamt: {0}", "Total: {0}", "Total: {0}";
    StatWon: "Gewonnen: {0}", "Won: {0}", "Ganadas: {0}";
    StatLost: "Verloren: {0}", "Lost: {0}", "Perdidas: {0}";
    StatDrawn: "Unentschieden: {0}", "Drawn: {0}", "Empatadas: {0}";
    StatRate: "Quote: {0}%", "Win rate: {0}%", "Victorias: {0}%";
    You: "(Sie)", "(you)", "(tú)";
    Challenge: "Herausfordern", "Challenge", "Retar";
    Accept: "Annehmen", "Accept", "Aceptar";
    Withdraw: "Zurückziehen", "Withdraw", "Retirar";
    ChallengeAll: "Alle herausfordern", "Challenge all", "Retar a todos";
    WithdrawAll: "Alle zurückziehen", "Withdraw all", "Retirar a todos";
    Disconnect: "Trennen", "Disconnect", "Desconectar";
    ActChallenge: "Diesen Spieler herausfordern", "Challenge this player", "Retar a este jugador";
    ActAccept: "Herausforderung annehmen", "Accept the challenge", "Aceptar el reto";
    ActWithdraw: "Herausforderung zurücknehmen", "Withdraw the challenge", "Retirar el reto";
    ActNone: "Keine Aktion möglich", "No action possible", "Ninguna acción posible";
    ActSelf: "Das sind Sie...", "That's you...", "Ese eres tú...";

    WaitingOpponent: "Warte auf Gegner...", "Waiting for opponent...", "Esperando al rival...";
    YouTag: " (du)", " (you)", " (tú)";
    VersusLine: "{0} gegen {1}", "{0} vs {1}", "{0} contra {1}";
    OpponentLeft: "{0} hat den Raum verlassen.", "{0} has left the room.", "{0} ha salido de la sala.";
    RematchAsked: "{0} möchte nochmal spielen!", "{0} wants to play again!", "¡{0} quiere jugar otra vez!";
    AfterHintOnline: "Enter: nochmal spielen    Esc: zurück zum Raum", "Enter: play again    Esc: back to room", "Enter: jugar otra vez    Esc: volver a la sala";
    ToRoom: "Zum Raum", "To room", "A la sala";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> Vec<usize> {
        (0..4).filter(|i| s.contains(&format!("{{{i}}}"))).collect()
    }

    #[test]
    fn every_text_exists_in_all_languages_with_the_same_placeholders() {
        for l in Lang::ALL {
            set_lang(l);
            for &k in Key::ALL {
                assert!(!tr(k).is_empty(), "{k:?} in {l:?}");
            }
        }
        for &k in Key::ALL {
            set_lang(Lang::De);
            let reference = placeholders(tr(k));
            for l in Lang::ALL {
                set_lang(l);
                assert_eq!(placeholders(tr(k)), reference, "{k:?} in {l:?}");
            }
        }
        // the language is global, so everything that changes it lives in this one test
        set_lang(Lang::En);
        assert_eq!(server_text("Der Server ist voll."), "The server is full.");
        assert_eq!(server_text("Verbindung fehlgeschlagen: timeout"), "Connection failed: timeout");
        assert_eq!(server_text("something else"), "something else");
        set_lang(Lang::De);
    }

    #[test]
    fn language_codes_round_trip() {
        for l in Lang::ALL {
            assert_eq!(Lang::from_code(l.code()), Some(l));
        }
        assert_eq!(Lang::from_code("fr"), None);
    }
}
