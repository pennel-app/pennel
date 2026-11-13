# Fase 4 — Pennel op de iPhone (ontwerp)

Stand: 2026-10-25. Bouwt op het hoofdontwerp (`2026-10-23-musli-design.md` §10). Het historische Cloud-serverontwerp staat in private beheerdersdocumentatie; actuele publieke appcontracten staan in [mobile implementation](../../mobile.md) en [device connection](../../device-connection.md).

## 1. Wat en waarom

Pennel op de iPhone doet drie dingen:

1. **Dicteren in elke app** via het Pennel-toetsenbord: tikken, praten, de opgekuiste tekst staat in het tekstveld. Ook zonder toetsenbord: Action Button, Tik achterop, een opdracht of een knop in het bedieningspaneel zet een dictaat op het klembord.
2. **Fysieke meetings opnemen** met de microfoon van de iPhone (een gesprek aan tafel, een klantbezoek), met dezelfde notities, samenvatting, transcript en vragen als op de computer.
3. **Dezelfde meetings op elk toestel** via Pennel Cloud-sync: een meeting die op de iPhone is opgenomen staat even later op de laptop, en omgekeerd.

**Waarom:** Ignace gebruikt Wispr Flow op de iPhone en Granola voor meetings. Pennel vervangt ze op de computer al; zolang de iPhone ontbreekt, blijft hij voor beide betalen. De iPhone is ook de plek waar fysieke meetings gebeuren, die de desktop niet kan opnemen.

**Klaar als:** Ignace dicteert een week lang op zijn iPhone alleen met Pennel (Slack, Mail, WhatsApp, ChatGPT), neemt een fysieke meeting op die binnen een minuut na afronden op zijn Mac staat, en heeft Wispr Flow niet meer nodig.

## 2. Doelen, niet-doelen, anti-doelen

**Doelen (v1)**
- Eén iPhone-app, gebouwd op dezelfde Rust-kern (`pennel-core`) als de desktop, met dezelfde providers, dezelfde opkuis- en samenvatlogica en hetzelfde Markdown-formaat.
- Pennel-toetsenbord (keyboard extension) met dicteerknop. Het toetsenbord neemt zelf niets op: de hoofdapp houdt een korte **dicteersessie** open en doet het werk (zoals Wispr Flow).
- Snel starten zonder de app te openen: App Intents voor Action Button, Tik achterop (via Opdrachten), Siri en bedieningspaneel.
- Fysieke meetings opnemen, ook met het scherm vergrendeld, met live transcript, notities, samenvatten, sjablonen, vragen en delen.
- Pennel Cloud: inloggen, sync van meetings en gehoste modellen, met **dezelfde kernmodules** (`cloud::account`, `cloud::sync`, `cloud::crypto`) en **hetzelfde protocol, ongewijzigd**.
- Werkt ook zonder account: Apple-dicteren (op het toestel) of een eigen API-sleutel.

**Niet-doelen (v1)**
- iPad-specifieke lay-out (de app draait op de iPad als iPhone-app), Apple Watch, Mac Catalyst.
- Audio van andere apps of telefoongesprekken opnemen (iOS laat dat niet toe).
- Lokale modellen (Whisper, Parakeet) op de iPhone — zie §5.4 en vraag 4.
- Commandomodus (selectie herschrijven), stijl per app, call-detectie en agendaregels: getoond maar uitgeschakeld met reden (§4.6).
- Sprekerherkenning met namen (vraagt het lokale sprekermodel).
- Audio synchroniseren (ook op desktop nog niet in Cloud v1).
- Een volwaardig typtoetsenbord met autocorrectie en suggesties.
- Betalen in de app (de beta van Pennel Cloud is gratis; zie risico R8).

**Anti-doelen (dan is het mislukt, ook als het werkt)**
- De microfoon staat open zonder dat de gebruiker het ziet. Zolang een sessie klaarstaat, is dat zichtbaar (oranje stip van iOS, Live Activity met stopknop) en stopt ze vanzelf.
- Audio wordt bewaard of verstuurd terwijl de sessie alleen klaarstaat. Alleen tussen "start" en "stop" van een dictaat telt audio.
- Het toetsenbord stuurt getypte tekens ergens heen. Het toetsenbord heeft geen netwerkcode, logt niets en schrijft geen getypte tekst naar de gedeelde map.
- Een eigen protocol of eigen sync voor de iPhone naast Pennel Cloud.
- Een tweede productlogica in Swift (opkuisen, samenvatten, chunking, zoeken, sync horen in de kern).
- Druk, generiek uiterlijk of een andere stijl dan de desktop.

## 3. Randvoorwaarden

- Productregels uit `CLAUDE.md`: privé als standaard, geen telemetrie, sleutels en tokens alleen in de Keychain, meetings zijn lokale Markdown, onbetrouwbare tekst kan geen acties starten.
- Zelfde visuele systeem uit `assets/brand/brand.md`: Paper/Background/Ink/Muted text/Line/Accent/Soft wave, Figtree en Instrument Serif, één accent per oppervlak, geen oranje op inkt (Soft wave op donker). Let op: `apps/macos/Sources/PennelKit/Theme.swift` gebruikt nog de verouderde Muted `#9B948A`; de iPhone volgt de brandgids (`#7A7268`).
- Nederlandse tekst eerst, en waar een scherm overlapt met de desktop dezelfde labels (§4).
- Licht: geen zware afhankelijkheden. SwiftUI, AVFoundation, Speech, FoundationModels, AppIntents, ActivityKit, WidgetKit, BackgroundTasks. Geen Firebase, geen analytics-SDK, geen derde-partij-UI-kit.
- **Doelwaarden:** app-download < 25 MB zonder modellen; toetsenbord-extensie < 30 MB geheugen in gebruik (iOS stopt toetsenborden rond 48–60 MB, afhankelijk van het toestel); van "stop" tot tekst in het veld < 2 s met Pennel Cloud of Groq.
- Minimumversie iOS 26 (mijn voorstel, vraag 3): Apple-dicteren (`SpeechAnalyzer`/`DictationTranscriber`), Apple Intelligence (`FoundationModels`) en `BGContinuedProcessingTask` bestaan pas vanaf iOS 26. `AudioRecordingIntent` en bedieningspaneelknoppen bestaan vanaf iOS 18.

## 4. Schermen en flows

De iPhone-app heeft vier tabbladen onderaan: **Meetings**, **Dicteren**, **Vraag**, **Instellingen**. Labels komen overeen met de desktop (`apps/desktop/src/views/*.ts`); waar ik een desktoplabel noem, neemt de bouwer de exacte tekst over uit die bestanden.

### 4.1 Meetings
- Bovenaan **Zoeken in meetings**. Daaronder de lijst per periode: **Vandaag**, **Gisteren**, **Afgelopen week**, **Eerder**. Met sync aan een klein sync-icoon per meeting (`Gesynchroniseerd`, `Wacht op sync`, `Conflictversie`), zoals in Cloud v1 deel 3.
- Grote knop **Nieuwe opname** (inkt, Soft wave-golfje), ook als opdracht en bedieningspaneelknop.
- **Opnemen (live):** titel (Instrument Serif, aanpasbaar), tijd, golfje, veld **Je notities**, ingeklapt **Live transcript**, knop **Stop opname**. Vergrendelen mag: opname loopt door, Live Activity en Dynamic Island tonen tijd en een stopknop.
- **Na afloop:** zoals desktop: notities aangevuld of samenvatting, beslissingen en actiepunten, sjabloon kiezen, **Samenvatten** / **Opnieuw genereren**.
- **Meeting openen:** segmentknop met dezelfde onderdelen als de desktop-meetingweergave (Notities, Transcript, vragen over deze meeting). Tik op een zin in het transcript → **Afspelen vanaf dit punt**. Menu **Delen**: **Kopieer opgemaakt**, **Kopieer als Markdown**, **Kopieer met transcript**, en het iOS-deelvenster (vervangt "Mail…/Deel…" van de Mac).
- **Spreker in een fysieke meeting** *(mijn voorstel)*: één microfoonspoor zonder "Ik/Anderen"-scheiding. Pennel schrijft het als spoor **Anderen** (bestaand formaat, desktop leest het zonder wijziging). Een apart label ("Iedereen") zou het Markdown-formaat en alle desktopversies raken; dat is vraag 6.

### 4.2 Dicteren (tabblad)
- **Dicteersessie**-kaart bovenaan met drie standen:
  - `Uit` — knop **Start sessie**.
  - `Klaar · nog 4:12` — knop **Stop sessie**. Uitleg: `De microfoon staat klaar voor het toetsenbord. Pennel luistert pas als je op de dicteerknop tikt.`
  - `Luistert…` / `Pennel verwerkt je dictaat.` (desktoptekst).
- Grote knop **Dicteer naar klembord**: opnemen in de app, resultaat op het klembord en in de geschiedenis. Dit is ook wat de Action Button doet.
- **Geschiedenis** (desktop: **Dictaten**, lege staat `Nog geen dictaten.`) met per item **Kopiëren** en **Opnieuw proberen**.

### 4.3 Vraag
De chat over alle meetings, zoals de desktopbibliotheek: placeholder `Vraag iets over al je meetings`, antwoorden met bronnen die naar de meeting springen.

### 4.4 Instellingen
Zelfde onderdelen en volgorde als de desktop: **Providers, Modellen, Dicteren, Meetings, Opnemen, Woordenlijst, Stijlen**, plus **Account** als laatste (Cloud v1). Op de iPhone is dat een lijst die naar subschermen gaat in plaats van tabs.
- **Providers:** dezelfde kaarten. Pennel Cloud, Apple-dicteren, Apple Intelligence en OpenAI-compatibele providers met eigen sleutel. CLI-agenten (Claude Code, Codex, …) staan er uitgeschakeld met `Werkt alleen op een computer.`
- **Modellen:** model per taak. Lokale modellen uitgeschakeld met `Lokale modellen komen later naar iPhone. Apple-dicteren werkt al zonder internet.`
- **Dicteren:** stand **Snel/Opgekuist**, taal, geschiedenis bewaren, en twee iPhone-subsecties die de desktopsneltoets vervangen:
  - **Toetsenbord:** status (toegevoegd? Volledige toegang?), stappen om het toe te voegen, knop **Open Instellingen**.
  - **Snel starten:** sessieduur (**5 min**, 15 min, 1 uur; standaard 5 min), en uitleg voor Action Button, Tik achterop en bedieningspaneel.
- **Meetings:** sjabloon, audio bewaren. Echo-onderdrukking uitgeschakeld met `Niet nodig op iPhone: Pennel neemt alleen de microfoon op.`
- **Opnemen:** call-herkenning, agenda en appregels uitgeschakeld met `Op iPhone neemt Pennel alleen op wanneer jij op Nieuwe opname tikt.`
- **Woordenlijst:** identiek.
- **Stijlen:** zichtbaar, stijl per app uitgeschakeld met `iOS laat Pennel niet zien in welke app je typt. Op iPhone geldt de standaardstijl.`
- **Account:** identiek aan Cloud v1 deel 3 (inloggen, gebruik, sync, herstelcode, toestellen, account verwijderen). Toestelnaam standaard `iPhone` (iOS geeft de echte naam niet meer vrij), aanpasbaar.

### 4.5 Het toetsenbord
Twee lay-outs in één toetsenbord, wisselen met een toets:
- **Dicteren** (standaard): een grote donkere pil met de vijf balkjes (Soft wave op inkt), daaronder 🌐 (volgend toetsenbord, verplicht), spatie, ⌫, return en **ABC**.
- **ABC:** een eenvoudig lettertoetsenbord (QWERTY; AZERTY instelbaar, vraag 5) zonder autocorrectie of suggesties. Nodig voor App Review (richtlijn 4.4.1: het toetsenbord moet ook zonder Volledige toegang tekst kunnen invoeren).

Gebaren zoals op de desktop: **ingedrukt houden** = praten tot loslaten; **tikken** = handenvrij tot je opnieuw tikt; ✕ = annuleren.

Standen en tekst:

| Stand | Wat het toetsenbord toont |
|---|---|
| Geen Volledige toegang | `Zet Volledige toegang aan om te dicteren. Typen werkt ook zonder.` + **Open Instellingen** |
| Sessie uit | Pil met label **Start Pennel**; daaronder `Pennel opent even en zet de microfoon klaar.` |
| Klaar | `Tik of hou ingedrukt om te praten` |
| Luistert | Bewegende balkjes, `Luistert…`, ✕ |
| Verwerkt | Balkjes zakken naar de pen-vorm met de stip, `Pennel verwerkt je dictaat.` |
| Fout | De Nederlandse fout uit de kern + **Opnieuw proberen** |
| Niet ingevoegd | **Laatste dictaat invoegen** (als het toetsenbord intussen weg was) |

**Flow "eerste dictaat"** (sessie uit):
1. Tik op **Start Pennel** → het toetsenbord opent `pennel://sessie/start`; Pennel komt naar voren.
2. Pennel zet de audiosessie aan, start de Live Activity `Pennel staat klaar` met een stopknop en toont: `Microfoon staat klaar. Ga terug met ◀ linksboven.`
3. De gebruiker tikt op de systeempijl linksboven en staat terug in zijn app. Er is geen API om automatisch terug te keren; de iOS-pijl is wat Apple aanraadt.
4. Tik op de pil → dictaat → tekst in het veld.

**Flow "volgend dictaat"** (sessie klaar): pil → praten → tekst. Geen app-wissel.

**Einde van de sessie:** na de gekozen duur zonder dictaat, via **Stop sessie** (app, Live Activity, bedieningspaneel), bij een telefoongesprek of als iOS de app stopt. De microfoon gaat dan echt dicht (oranje stip weg).

### 4.6 Snel starten zonder toetsenbord
App Intents (ook zichtbaar in Opdrachten en via Siri):

| Opdracht | Doet | Waar |
|---|---|---|
| **Dicteer naar klembord** | Pennel opent met de opnameknop actief; na stoppen staat de tekst op het klembord en in de geschiedenis | Action Button, Tik achterop, bedieningspaneel, Siri |
| **Start dicteersessie** | Zet de sessie klaar voor het toetsenbord | idem |
| **Stop dicteersessie** | Sluit de microfoon | idem, en knop in de Live Activity |
| **Start opname** | Nieuwe fysieke meeting, live-scherm | idem |
| **Stop opname** | Rondt de lopende meeting af | idem, en knop in de Live Activity |

*Mijn voorstel:* alle **start**-opdrachten openen Pennel even op de voorgrond. iOS laat het starten van een microfoonopname vanuit de achtergrond niet betrouwbaar toe (zelfs `AudioRecordingIntent` faalt volgens ontwikkelaars met "Target is not foreground"). **Stop**-opdrachten werken vanuit de achtergrond. Taak 11 test of starten zonder voorgrond toch lukt; zo ja, dan vervalt de app-wissel voor die opdracht.

Tik achterop is een iOS-instelling (Toegankelijkheid → Aanraken → Tik achterop) die een opdracht start; Pennel legt dat uit in **Snel starten**, maar kan het niet zelf instellen.

### 4.7 Eerste start
Kort, zoals de desktop-welkom (`Drie dingen en je kan dicteren in elke app.`):
1. Microfoon toestaan.
2. Spraak kiezen: **Pennel Cloud** (inloggen), **Apple-dicteren** (op dit toestel, geen account) of **Eigen provider**.
3. Toetsenbord toevoegen en Volledige toegang aanzetten (met uitleg wat dat wel en niet betekent: `Volledige toegang laat het toetsenbord met de Pennel-app praten. Het toetsenbord gebruikt geen internet en bewaart niet wat je typt.`).
4. Optioneel: Action Button instellen.

## 5. Hoe het werkt (beslist, tenzij "mijn voorstel")

### 5.1 Kern voor iOS
- De kern bouwt voor `aarch64-apple-ios` (toestel) en `aarch64-apple-ios-sim` (simulator op Apple Silicon). Geen Intel-simulator.
- Een nieuw script `scripts/build-core-ios.sh` maakt `apps/ios/Frameworks/PennelCoreFFI.xcframework` met beide slices en genereert de Swift-bindingen, net zoals `scripts/build-core.sh` dat voor de Mac doet. Het Mac-script blijft ongewijzigd.
- **Lokale modellen achter een Cargo-feature** *(mijn voorstel)*: `local-models` (standaard aan) zet `transcribe-cpp` aan. De iOS-build bouwt zonder. Belangrijk: de uniffi-interface blijft **identiek**; zonder de feature geven de lokale functies de Nederlandse fout `Lokale modellen zijn niet beschikbaar op dit toestel.` Alleen `local_engine.rs` en `diarize.rs` gebruiken `transcribe_cpp`, dus de ingreep is klein.
  - Gemeten op de Mac (release, gestript): `libpennel_core.dylib` met whisper.cpp en Metal = 10,7 MB; de hele Mac-app is 17 MB. Zonder `transcribe-cpp` verwacht ik ruwweg de helft; taak 2 meet het voor iOS en zet het cijfer in `docs/ios.md`.
  - Haalbaarheid later: `transcribe-cpp-sys` herkent `ios` al als Apple-doel en bedt de Metal-bibliotheek in (`GGML_METAL_EMBED_LIBRARY`). Lokale modellen zijn dus een feature-schakelaar en een geheugenafweging, geen herschrijving.
- Desktopmodules die op iOS niets doen (call-detectie, gebaren, plakwachtrij, CLI-agenten) compileren mee maar worden niet gebruikt. CLI-agenten vinden op iOS geen uitvoerbaar bestand en worden in de UI uitgeschakeld getoond.
- Gegevensmap van de kern: `Library/Application Support/Pennel` in de **eigen** container van de app. Meetingmap: `Documents/Meetings`, zichtbaar in de Bestanden-app ("Op mijn iPhone › Pennel"). Zo blijven meetings gewone Markdown-bestanden.
- **Niet in de App Group** *(beslist)*: de kern houdt bestandsvergrendelingen (`DirectoryLease`) en SQLite open. Een opgeschorte app met een vergrendeling in een gedeelde container wordt door iOS gestopt (crashcode `0xdead10cc`), en twee processen kunnen dezelfde map niet tegelijk vergrendelen. Daarom draait de kern **alleen in de hoofdapp**.

### 5.2 Toetsenbord ↔ app
- Het toetsenbord linkt de Rust-kern **niet** (geheugen, vergrendeling, en het heeft geen netwerk nodig). Het kent alleen een klein Swift-pakket met het overdrachtsprotocol.
- Gedeelde App Group `group.app.pennel` met één map `handoff/` met kleine JSON-bestanden, atomisch weggeschreven (tijdelijk bestand + hernoemen), zonder vergrendelingen:
  - `session.json` (app schrijft): stand (`off`, `ready`, `listening`, `processing`), `expires_at`, `heartbeat_at`, en tijdens luisteren een niveau voor de balkjes.
  - `command.json` (toetsenbord schrijft): `{id, action: start|stop|cancel, at}`.
  - `result.json` (app schrijft): `{id, text | error, at}`, en wordt gewist na invoegen of na 10 minuten.
- Seintjes via Darwin-notificaties (`app.pennel.handoff.command`, `app.pennel.handoff.session`, `app.pennel.handoff.result`); die dragen geen inhoud, alleen "kijk opnieuw".
- Het toetsenbord voegt een resultaat **precies één keer** in, en alleen als het `id` van het eigen commando komt. Is het toetsenbord intussen weg, dan blijft de tekst in de geschiedenis en biedt het toetsenbord **Laatste dictaat invoegen**.
- Een sessie geldt als dood als `heartbeat_at` ouder is dan 5 s (de app is gestopt); het toetsenbord toont dan "Sessie uit".
- De app opent vanuit het toetsenbord via de responder-keten (`open(_:)`); dat vraagt Volledige toegang (sinds iOS 26). Deze code staat op één plek, zodat ze uit kan als App Review bezwaar maakt (R1); het toetsenbord toont dan `Open Pennel en tik op Start sessie.`

### 5.3 Dicteersessie in de app
- `AVAudioSession` in `.playAndRecord` met `.mixWithOthers`, achtergrondmodus `audio`. Zolang de sessie klaarstaat, draait de microfoon-engine; samples worden **meteen weggegooid** tot er een startcommando komt. Pas dan gaan samples (16 kHz mono Float32, zoals op de desktop) in een buffer; bij stop roept de app `PennelEngine::dictate` aan op een achtergrondthread.
- Live Activity (en Dynamic Island) tijdens de sessie: `Pennel staat klaar · 4:12` met **Stop**. Tijdens een dictaat bewegen de balkjes.
- Onderbrekingen (telefoon, Siri, alarm) beëindigen de sessie netjes; de Live Activity zegt `Sessie gestopt door een oproep.`
- Een meeting die opneemt en een dictaat delen één microfoonstroom; het dictaatstuk wordt via de bestaande `MeetingSession::set_dictating`/`mark_dictation` uit het meetingtranscript gehouden, zoals op de desktop.

### 5.4 Spraak en taalmodellen op de iPhone
| Taak | Pennel Cloud | Eigen provider | Op het toestel |
|---|---|---|---|
| Dicteren (spraak) | ja (`pennel-auto`) | ja | **Apple-dicteren** (`DictationTranscriber`, iOS 26) |
| Meetings (spraak) | ja | ja | Apple-dicteren |
| Opkuisen, samenvatten, vragen | ja | ja | **Apple Intelligence** (`FoundationModels`, iPhone 15 Pro en nieuwer met Apple Intelligence aan) |
| Lokale Whisper/Parakeet | — | — | niet in v1 (vraag 4) |

- De Swift-adapters voor Apple-dicteren en Apple Intelligence bestaan al voor de Mac (`apps/macos-platform/Sources/PennelMacPlatform/AppleSpeech.swift`, `FoundationLanguageModel.swift`) en worden voor iOS hergebruikt achter dezelfde kern-traits `SystemTranscriber` en `SystemLanguageModel`.
- **Standaard bij een nieuwe gebruiker** *(mijn voorstel)*: ingelogd → Pennel Cloud voor de lege taken (zoals Cloud v1). Niet ingelogd → Apple-dicteren voor spraak, Apple Intelligence voor opkuisen als het toestel dat heeft, anders stand **Snel**. Zo werkt Pennel meteen, privé en zonder sleutel.
- **Alleen met Pennel Cloud:** sync tussen toestellen en gehoste modellen zonder eigen sleutel. Al het andere werkt zonder account.

### 5.5 Meetings opnemen
- Microfoon via `AVAudioEngine` (stemverwerking aan of uit: de bouwer test beide in een echte vergaderzaal), 16 kHz mono naar `MeetingSession::push_audio_at(Speaker::Others, …)` met tijdstempels, zodat onderbrekingen een gat geven in plaats van verschoven tekst.
- De app schrijft het archief als AAC in `anderen.m4a` (AVFoundation), zoals de Mac `ik.m4a`/`anderen.m4a` schrijft. Audio blijft op het toestel (Cloud v1 synct geen audio).
- Opnemen loopt door met vergrendeld scherm (achtergrondmodus `audio`). Na **Stop opname** rondt Pennel af (laatste stukken transcriberen, samenvatten). Als de gebruiker de app verlaat, loopt dat verder in een `BGContinuedProcessingTask` met voortgang in het systeem (iOS 26), en anders bij de volgende start via de bestaande "Opnieuw proberen".
- Na afronden gaat de meeting meteen in de sync (§5.6). Een meeting die nog opneemt, synct niet (Cloud v1-regel).

### 5.6 Pennel Cloud en sync
- Hergebruik zonder wijziging: `cloud::account` (device-code login), `cloud::crypto` (kluissleutel, herstelcode), `cloud::sync` (`cloud_sync_run`) en `ProviderKind::PennelCloud`. Platform-string bij inloggen: `ios`. Het protocol en de serverendpoints veranderen niet.
- `SecretStore` met de Keychain, toegang `AfterFirstUnlockThisDeviceOnly`, zodat sync en gehoste modellen ook werken met vergrendeld scherm tijdens een opname, en sleutels niet in back-ups of op een ander toestel belanden.
- Inloggen: de app opent `verification_uri` (met de code erin) in Safari via `ASWebAuthenticationSession` of de standaardbrowser; de magic link opent in Mail, de gebruiker bevestigt, de app pollt zoals de desktop.
- Wanneer sync draait: bij het openen van de app, na elke eigen schrijfactie (afronden, notities, hernoemen, samenvatten), elke 3 minuten zolang de app op de voorgrond staat, en opportunistisch via `BGAppRefreshTask`. iOS bepaalt wanneer die laatste loopt; Pennel belooft geen achtergrondsync.
- Waarschuwing "map synct al via iCloud" is op de iPhone niet van toepassing: de meetingmap staat lokaal.

### 5.7 Projectstructuur
- `apps/ios/Pennel.xcodeproj` met drie targets: **Pennel** (app), **PennelKeyboard** (keyboard extension), **PennelWidgets** (Live Activity + bedieningspaneelknoppen).
- `apps/ios/PennelMobileKit/` — Swift-pakket zonder UIKit-afhankelijkheden in de logica, zodat `swift test` op de Mac draait: overdrachtsprotocol, sessieklok, resampler, merkwaarden (kleuren, lettertypes uit `assets/brand/fonts/`), Nederlandse teksten.
- Bundel-id's *(mijn voorstel, bouwer mag aanpassen)*: `app.pennel.ios`, `app.pennel.ios.keyboard`, `app.pennel.ios.widgets`, App Group `group.app.pennel`.

## 6. Overgelaten aan de bouwer
- Xcode-project handmatig of via een generator (zonder extra runtime-afhankelijkheid).
- Exacte lay-out van het lettertoetsenbord, toetsgroottes, haptiek.
- Hoe het niveau voor de balkjes gedeeld wordt (bestand of gedeelde `UserDefaults`), zolang het binnen het geheugenbudget blijft.
- Stemverwerking aan of uit voor fysieke meetings (testen in een echte vergaderzaal).
- Volgorde van de UI-schermen binnen een taak, zolang labels en volgorde van §4 kloppen.

## 7. Risico's

| # | Risico | Gevolg | Wat we doen |
|---|---|---|---|
| R1 | **App Review 4.4.1**: toetsenborden "mogen geen andere apps openen behalve Instellingen". Ons toetsenbord opent de eigen app. Wispr Flow, Superwhisper en Willow doen hetzelfde en staan in de App Store, maar dat is geen garantie. | Afwijzing van het toetsenbord | Het openen staat op één plek en kan uit (dan: `Open Pennel en tik op Start sessie.`); Action Button en bedieningspaneel blijven werken. Review-notitie legt uit dat het de eigen app is, om de microfoon te mogen gebruiken. TestFlight-review eerst. |
| R2 | **4.4.1**: het toetsenbord moet werken zonder Volledige toegang en zonder netwerk. | Afwijzing | Lettertoetsenbord werkt altijd; dicteren toont dan uitleg. Het toetsenbord gebruikt nooit netwerk. |
| R3 | **2.5.4/2.5.14 achtergrondaudio**: microfoon openhouden voor een toetsenbord kan als misbruik gezien worden. | Afwijzing of latere verwijdering | Duidelijke opt-in, zichtbare Live Activity, korte standaardduur (5 min), audio pas na start. Review-notitie met video. |
| R4 | iOS verandert de regels (26.4 nam de host-app-id al weg; starten vanuit de achtergrond is onbetrouwbaar). | Flows breken bij een iOS-update | Geen privé-API's; alleen de iOS-terugpijl en voorgrond-start. Taak 11 test de achtergrondstart per iOS-versie. |
| R5 | Toetsenbordgeheugen (~48–60 MB). | iOS sluit het toetsenbord | Geen Rust, geen grote afbeeldingen, alleen de nodige lettertypes; geheugen gemeten in taak 8. |
| R6 | `0xdead10cc` door vergrendelingen in de App Group. | Crashes in de achtergrond | Kern buiten de App Group (§5.1); overdracht zonder vergrendelingen. |
| R7 | Kruiscompilatie (cmake, C++ in `transcribe-cpp`) voor iOS. | Bouwproblemen | iOS-build zonder `local-models`; lokaal later. |
| R8 | **Betalen (3.1.1)**: wordt Pennel Cloud betalend, dan vraagt Apple in-app-aankoop of een uitzondering. | Werk en marge na de beta | v1 noemt geen prijzen en linkt niet naar betalen. Beslissing bij commercialisatie. |
| R9 | **Reviewer-account (2.1)**: inloggen met magic link werkt niet voor een reviewer zonder mailbox. | Review van sync niet mogelijk | De app werkt volledig zonder account (Apple-dicteren). Voor Cloud: een reviewaccount met een mailbox die we delen; als dat niet volstaat, een vaste testcode op de server (in `pennel-cloud`, buiten deze repo). |
| R10 | Privacylabel in App Store Connect: met Cloud verzamelt Pennel een e-mailadres. | Verkeerde claim | Label: e-mail (gekoppeld, alleen voor account), geen tracking, geen analytics. Websitecopy pas aanpassen bij release. |
| R11 | Microfoon-indicator tijdens "klaar" schrikt gebruikers af. | Wantrouwen | Eerlijke uitleg bij de eerste sessie; standaard 5 min. |
| R12 | Lange meetings en geheugen/batterij op oudere iPhones. | Opname stopt | Test van 60 minuten op het oudste ondersteunde toestel (acceptatie T14). |

## 8. Haalbaarheid (gecontroleerd op 2026-10-25)
- Xcode 27 met de iOS 27 SDK staat op de bouwmachine; de Rust-doelen voor iOS zijn er nog niet (`rustup target add aarch64-apple-ios aarch64-apple-ios-sim`, taak 1). Een echte iOS-build van de kern is dus nog niet gedaan.
- `transcribe-cpp-sys 0.4.1` behandelt `ios` als Apple-doel met Metal; de rest van de kern is pure Rust of gebundelde C (`rusqlite` bundled, `ureq` met rustls) en bouwt normaal voor iOS.
- Apple: toetsenborden hebben geen microfoon, ook niet met Volledige toegang; er is geen API om na de app-wissel automatisch terug te keren (DTS raadt de systeempijl aan); de host-app-id is sinds iOS 26.4 leeg; `AudioRecordingIntent` (iOS 18+) vraagt een Live Activity zolang er opgenomen wordt.

## 9. Keuzes die ik maakte (te controleren)
1. Het toetsenbord neemt niets op en linkt de kern niet; de app houdt een dicteersessie open (Wispr-patroon).
2. De kern draait alleen in de app, met de gegevens in de eigen container; de App Group bevat alleen kleine overdrachtsbestanden zonder vergrendelingen.
3. Standaardduur van de sessie: 5 minuten na het laatste dictaat; keuze 5 min, 15 min, 1 uur.
4. Start-opdrachten openen de app even; stop-opdrachten werken in de achtergrond.
5. Lokale modellen achter een Cargo-feature; uit op iOS in v1, met identieke uniffi-interface.
6. Fysieke meetings schrijven één spoor als **Anderen**, zodat het Markdown-formaat niet verandert.
7. Zonder account: Apple-dicteren en Apple Intelligence als standaard; met account Pennel Cloud.
8. Desktopfuncties die iOS niet toelaat staan uitgeschakeld met reden, niet verborgen.
9. Sync hergebruikt Cloud v1 ongewijzigd, met platform `ios`.
10. Minimum iOS 26.
11. Toetsenbord met dicteer-lay-out en een eenvoudig lettertoetsenbord zonder autocorrectie.

## 10. Vragen voor Ignace
Alleen vragen waarvan het antwoord verandert wat gebouwd wordt. Zonder antwoord geldt de standaard.

1. **Apple Developer Program (€99/jaar) nu nemen?** Nodig voor TestFlight, de App Store, en stabiel signeren van een toetsenbord met App Group op je iPhone; dezelfde inschrijving dekt de Developer ID voor de Mac. Zonder: alleen een gratis build op je eigen iPhone die elke 7 dagen opnieuw gesigneerd moet worden. **Standaard: ja, nu.**
2. **Distributie v1:** alleen TestFlight (jij en collega's), of meteen de App Store? **Standaard: TestFlight eerst; App Store na een week eigen gebruik en acceptatie.**
3. **Minimum iOS-versie:** 26 (Apple-dicteren en Apple Intelligence op het toestel) of 18 (meer toestellen, maar zonder die twee, dus altijd Cloud of eigen sleutel)? **Standaard: iOS 26.**
4. **Lokale modellen (Whisper/Parakeet) in de eerste iPhone-versie?** Kost +~6 MB download, honderden MB geheugen per model en meer testwerk; Apple-dicteren werkt al offline. **Standaard: nee in v1, later als feature-schakelaar.**
5. **Lettertoetsenbord: QWERTY of AZERTY als standaard?** **Standaard: QWERTY, AZERTY instelbaar.**
6. **Fysieke meeting als "Anderen" of als nieuw label "Iedereen"?** Een nieuw label vraagt een kleine kern- en desktopaanpassing vóór iPhone-meetings syncen. **Standaard: "Anderen".**
