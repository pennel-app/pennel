import Foundation

public struct KeyboardEmoji: Identifiable, Equatable, Sendable {
    public let symbol: String
    public let english: String
    public let dutch: String
    public var id: String { symbol }

    public init(_ symbol: String, _ english: String, _ dutch: String) {
        self.symbol = symbol
        self.english = english
        self.dutch = dutch
    }

    public func label(language: KeyboardInterfaceLanguage) -> String {
        language == .dutch ? dutch : english
    }
}

public enum KeyboardEmojiSearch {
    public static let catalogue: [KeyboardEmoji] = [
        .init("😀", "happy smile face", "blij lach gezicht"),
        .init("😃", "happy smile excited", "blij lachen enthousiast"),
        .init("😄", "smile laugh happy", "glimlach lachen blij"),
        .init("😁", "grin smile teeth", "grijns glimlach tanden"),
        .init("😆", "laugh funny", "lachen grappig"),
        .init("😂", "tears laugh funny", "tranen lachen grappig"),
        .init("🤣", "rolling laugh funny", "schaterlachen grappig"),
        .init("😊", "smile kind thanks", "glimlach lief bedankt"),
        .init("🙂", "smile face", "glimlach gezicht"),
        .init("😉", "wink joke", "knipoog grapje"),
        .init("😍", "love heart eyes", "liefde hart ogen"),
        .init("🥰", "love affection happy", "liefde lief blij"),
        .init("😘", "kiss love", "kus liefde"),
        .init("😎", "cool sunglasses", "cool zonnebril"),
        .init("🤔", "think question", "denken vraag"),
        .init("🤨", "skeptical question", "twijfel vraag"),
        .init("😐", "neutral face", "neutraal gezicht"),
        .init("🙃", "upside down irony", "ondersteboven ironie"),
        .init("😅", "sweat relief laugh", "zweet opluchting lachen"),
        .init("😬", "awkward nervous", "ongemakkelijk zenuwachtig"),
        .init("😢", "cry sad tear", "huilen verdrietig traan"),
        .init("😭", "cry sad tears", "huilen verdrietig tranen"),
        .init("😔", "sad disappointed", "verdrietig teleurgesteld"),
        .init("😴", "sleep tired", "slapen moe"),
        .init("😤", "angry frustrated", "boos gefrustreerd"),
        .init("😡", "angry mad", "boos kwaad"),
        .init("😱", "shocked scared", "geschrokken bang"),
        .init("🥳", "party celebrate birthday congratulations", "feest vieren verjaardag gefeliciteerd proficiat"),
        .init("🤩", "star excited amazing", "ster enthousiast geweldig"),
        .init("🤗", "hug support", "knuffel steun"),
        .init("🫡", "salute respect", "groet respect"),
        .init("🤫", "quiet secret", "stil geheim"),
        .init("🤦", "facepalm mistake", "facepalm fout"),
        .init("🤷", "shrug unsure", "schouders weet niet"),
        .init("👍", "thumb up yes good agree", "duim omhoog ja goed akkoord"),
        .init("👎", "thumb down no bad", "duim omlaag nee slecht"),
        .init("👏", "clap applause congratulations", "klappen applaus gefeliciteerd proficiat"),
        .init("🙌", "celebrate hands hooray", "vieren handen hoera"),
        .init("🙏", "thanks please pray", "bedankt alsjeblieft bidden"),
        .init("👋", "wave hello goodbye", "zwaaien hallo dag afscheid"),
        .init("👌", "okay perfect", "oké perfect"),
        .init("🤞", "fingers crossed luck", "duimen geluk"),
        .init("✌️", "peace two", "vrede twee"),
        .init("💪", "strong muscle support", "sterk spier steun"),
        .init("❤️", "red heart love", "rood hart liefde"),
        .init("🧡", "orange heart love", "oranje hart liefde"),
        .init("💛", "yellow heart love", "geel hart liefde"),
        .init("💚", "green heart love", "groen hart liefde"),
        .init("💙", "blue heart love", "blauw hart liefde"),
        .init("💜", "purple heart love", "paars hart liefde"),
        .init("🖤", "black heart", "zwart hart"),
        .init("🤍", "white heart", "wit hart"),
        .init("💔", "broken heart sad", "gebroken hart verdrietig"),
        .init("💕", "hearts love", "harten liefde"),
        .init("💯", "hundred perfect agree", "honderd perfect akkoord"),
        .init("🔥", "fire hot great", "vuur heet geweldig"),
        .init("✨", "sparkles stars magic", "glitters sterren magie"),
        .init("⭐", "star favorite", "ster favoriet"),
        .init("🎉", "party celebration congratulations", "feest vieren gefeliciteerd proficiat"),
        .init("🎂", "birthday cake", "verjaardag taart"),
        .init("🎁", "gift present", "cadeau geschenk"),
        .init("🎈", "balloon birthday party", "ballon verjaardag feest"),
        .init("☀️", "sun sunny weather", "zon zonnig weer"),
        .init("🌧️", "rain weather", "regen weer"),
        .init("🌈", "rainbow pride", "regenboog trots"),
        .init("🌙", "moon night", "maan nacht"),
        .init("🌻", "sunflower flower", "zonnebloem bloem"),
        .init("🌹", "rose flower love", "roos bloem liefde"),
        .init("🍀", "clover luck", "klaver geluk"),
        .init("🐶", "dog puppy", "hond puppy"),
        .init("🐱", "cat kitten", "kat kitten"),
        .init("🐾", "paw pet animal", "poot huisdier dier"),
        .init("☕", "coffee tea drink", "koffie thee drinken"),
        .init("🍵", "tea drink", "thee drinken"),
        .init("🍺", "beer drink cheers", "bier drinken proost"),
        .init("🥂", "cheers champagne celebrate", "proost champagne vieren"),
        .init("🍷", "wine drink", "wijn drinken"),
        .init("🍕", "pizza food", "pizza eten"),
        .init("🍟", "fries chips food", "friet frieten eten"),
        .init("🍔", "burger food", "hamburger eten"),
        .init("🍰", "cake dessert", "taart gebak dessert"),
        .init("🏠", "home house", "thuis huis"),
        .init("💼", "work office", "werk kantoor"),
        .init("💻", "computer laptop code", "computer laptop code"),
        .init("📱", "phone mobile", "telefoon mobiel"),
        .init("📞", "call phone", "bellen telefoon"),
        .init("📧", "email mail", "email e-mail post"),
        .init("📝", "note write meeting", "notitie schrijven vergadering"),
        .init("📅", "calendar date meeting", "kalender datum afspraak vergadering"),
        .init("⏰", "alarm time reminder", "wekker tijd herinnering"),
        .init("✅", "check done yes", "vink klaar ja"),
        .init("❌", "cross no cancel", "kruis nee annuleren"),
        .init("⚠️", "warning attention", "waarschuwing aandacht"),
        .init("❓", "question help", "vraag hulp"),
        .init("💡", "idea light bulb", "idee lamp"),
        .init("🚀", "rocket launch fast", "raket lancering snel"),
        .init("✈️", "plane travel holiday", "vliegtuig reizen vakantie"),
        .init("🚗", "car drive travel", "auto rijden reizen"),
        .init("🚲", "bike bicycle ride", "fiets fietsen"),
        .init("🚆", "train travel", "trein reizen"),
        .init("🏃", "run sport", "lopen hardlopen sport"),
        .init("⚽", "football soccer sport", "voetbal sport"),
        .init("🏖️", "beach holiday", "strand vakantie"),
        .init("🎵", "music song", "muziek lied"),
        .init("🎮", "game gaming controller", "spel gamen controller"),
        .init("👨‍👩‍👧‍👦", "family parents children", "familie ouders kinderen gezin"),
        .init("🇧🇪", "belgium flag", "belgië vlag"),
        .init("🇳🇱", "netherlands flag", "nederland vlag"),
        .init("🇬🇧", "united kingdom british english flag", "verenigd koninkrijk brits engels vlag"),
    ]

    public static func search(_ query: String, recent: [String] = [], limit: Int = 60) -> [KeyboardEmoji] {
        let limit = max(0, min(limit, catalogue.count))
        guard limit > 0 else { return [] }
        let terms = folded(String(query.prefix(128))).split(whereSeparator: \.isWhitespace)
        var seen: Set<String> = []
        let recentEntries = recent.prefix(24).compactMap { symbol in catalogue.first { $0.symbol == symbol } }
        return Array((recentEntries + catalogue).filter { entry in
            guard seen.insert(entry.symbol).inserted else { return false }
            let haystack = folded(entry.symbol + " " + entry.english + " " + entry.dutch)
            return terms.allSatisfy { haystack.contains($0) }
        }.prefix(limit))
    }

    private static func folded(_ value: String) -> String {
        value.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "nl"))
    }
}

public enum KeyboardToolPreferences {
    public static let recentEmojiKey = "keyboardRecentEmoji"

    public static func recentEmoji(in defaults: UserDefaults?) -> [String] {
        let allowed = Set(KeyboardEmojiSearch.catalogue.map(\.symbol))
        var seen: Set<String> = []
        return Array((defaults?.stringArray(forKey: recentEmojiKey) ?? []).filter {
            allowed.contains($0) && seen.insert($0).inserted
        }.prefix(24))
    }

    public static func recordEmoji(_ value: String, in defaults: UserDefaults?) {
        guard KeyboardEmojiSearch.catalogue.contains(where: { $0.symbol == value }) else { return }
        defaults?.set(Array(([value] + recentEmoji(in: defaults).filter { $0 != value }).prefix(24)), forKey: recentEmojiKey)
    }
}
