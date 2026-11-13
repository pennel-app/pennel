"""Fresh authored Dutch meetings for managed Cloud and local Decider profiles.

This module supplies data and independent judgments, not model output or scoring.
It never selects a transport or grants qualification. Related variants stay in one split.
"""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

KINDS = ('action', 'decision', 'open_question', 'changed_commitment')
STATES = ('none', 'tentative', 'confirmed', 'retracted', 'unresolved', 'changed', 'unknown', 'negated')
SUPPORT = ('supported', 'absent', 'unclear')

CASE_STATES = (
    {'action': 'confirmed'}, {'decision': 'confirmed'}, {'open_question': 'unresolved'},
    {'decision': 'changed', 'changed_commitment': 'changed'}, {'action': 'tentative'},
    {'decision': 'negated'}, {}, {'action': 'retracted', 'changed_commitment': 'retracted'}, {},
    {'action': 'confirmed', 'open_question': 'unresolved'}, {'decision': 'tentative'},
    {'action': 'negated'}, {'decision': 'confirmed', 'action': 'confirmed'}, {},
    {'action': 'unknown'}, {'open_question': 'unresolved'},
)
_PRESENT = (
    ('action',), ('decision',), ('open_question',), ('decision', 'changed_commitment'),
    ('action',), (), (), ('action', 'changed_commitment'), (), ('action', 'open_question'),
    ('decision',), (), ('decision', 'action'), (), ('action',), ('open_question',),
)
CASE_SUPPORT = tuple({kind: 'supported' if kind in present else 'absent' for kind in KINDS} for present in _PRESENT)

# Each row is a separate fictional episode: topic, firm-work phrase, second-work
# phrase, selected alternative, superseded alternative, descriptive fact, location.
# These are not translations or continuations of another split's episodes.
FAMILIES = {
    'development': (
        ('een pluktuin voor scholieren', 'de paden langs de bessenstruiken afzetten', 'de gieters bij de ingang tellen', 'brede looppaden', 'smalle looppaden', 'De bodem bestaat uit donkere klei', 'naast de lage stenen muur'),
        ('een cursus boekbinden', 'de vouwmessen op de werktafels leggen', 'de linnenstroken op lengte meten', 'een harde rug', 'een soepele rug', 'Het papier heeft een korrelige structuur', 'in de ondiepe lade'),
        ('een stiltewandeling', 'de markering bij de kruising nalopen', 'de drinkflessen voor de gidsen klaarleggen', 'de route door het open veld', 'de route langs de weg', 'Het terrein heeft een lichte helling', 'onder het houten afdak'),
        ('een kledingruil voor studenten', 'de kapstokken aan de wand vastmaken', 'de kledingrekken per maat verdelen', 'drie vaste tijdvakken', 'één doorlopend tijdvak', 'De zaal heeft een brede zijdeur', 'naast het grote spiegelvlak'),
        ('een proef met regentonnen', 'de koppelingen onder de bakken nakijken', 'de maatstrepen op de tonnen zetten', 'een gesloten overloop', 'een open overloop', 'De bakken zijn van donkergroen kunststof', 'in het metalen krat'),
        ('een buurtvoorstelling met poppen', 'de kleine lampen boven het doek richten', 'de handpoppen per scène groeperen', 'een zwart achterdoek', 'een wit achterdoek', 'Het podium heeft een lage voorkant', 'achter het rode scherm'),
        ('een bezoek aan een melkveebedrijf', 'de overschoenen bij de deur aanvullen', 'de luisterkastjes op batterij controleren', 'een kleine bezoekersgroep', 'een grote bezoekersgroep', 'De ontvangstruimte kijkt uit op de stal', 'aan de voet van de trap'),
        ('een demonstratie glasblazen', 'de beschermbrillen langs de werkbank leggen', 'de zitplaatsen vooraan nummeren', 'een ronde opstelling', 'een rechte opstelling', 'De werkbank heeft een hittebestendig blad', 'in de kast met de schuifdeur'),
        ('een opleiding verkeersregelaars', 'de oefenhesjes op de stoelen leggen', 'de portofoons voor de proefrit laden', 'een korte oefenronde', 'een lange oefenronde', 'Het oefenplein is rechthoekig', 'bij de doorgang van de loods'),
        ('een nestkastenproject', 'de ophangbeugels aan de achterplaat zetten', 'de dakplaten op breedte controleren', 'een ondiepe aanvliegplank', 'een diepe aanvliegplank', 'Het hout is licht van kleur', 'boven op de lage stelling'),
        ('een cursus sterrenkaarten tekenen', 'de linialen bij de tekentafels leggen', 'de transparante bladen op formaat snijden', 'een donkere achtergrond', 'een lichte achtergrond', 'De lesruimte heeft een hoog plafond', 'tussen de twee blauwe laden'),
        ('een oefenmiddag rolstoelbasketbal', 'de banden van de sportstoelen oppompen', 'de beschermhoezen per stoel uitzoeken', 'een halve speelhelft', 'een volledige speelhelft', 'De vloer heeft een gladde toplaag', 'tegen de noordelijke zijwand'),
    ),
    'calibration': (
        ('de fietsenstalling bij een wijkgebouw', 'de losse beugel aan het rek vastzetten', 'de reflectoren op de buitenrand controleren', 'genummerde vakken', 'vrije vakken', 'De overkapping is drie meter hoog', 'achter de gele deur'),
        ('de energieaanpak van een bibliotheek', 'de tochtstrip onder de buitendeur vervangen', 'de instellingen van de tijdschakelaar nalopen', 'warm licht in de leeshoek', 'koel licht in de leeshoek', 'De glaswand bestaat uit zes panelen', 'in het vak onder de balie'),
        ('de allergenenhoek van een buurtkeuken', 'de kruidenpotten opnieuw etiketteren', 'de voorraad lepels per kleur tellen', 'pictogrammen op de bakken', 'letters op de bakken', 'De spoelbak heeft twee diepe delen', 'naast de smalle koelkast'),
        ('de dienstwisseling in een busgarage', 'de routeborden boven de werkplekken ophangen', 'de laadkabels bij het achterhek controleren', 'een vaste wisselplek', 'een wisselende wisselplek', 'De toegangspoort heeft vier schuifdelen', 'onder de overdekte doorgang'),
        ('de winterpaden van een kinderboerderij', 'het buitenhek opnieuw afstellen', 'de strooibakken langs het pad aanvullen', 'een verharde doorsteek', 'een zandige doorsteek', 'De schuilstal heeft een rieten dak', 'in de kist bij de hooiberg'),
        ('de geluidsproef van een muziekpodium', 'de microfoonkabels doormeten', 'de stekkers van het mengpaneel nalopen', 'een lage geluidsgrens', 'een hoge geluidsgrens', 'De achterwand heeft zwarte panelen', 'op de plank boven de versterker'),
        ('de groepsindeling van een zwemschool', 'de reserveringen voor de kleine groep controleren', 'de polsbandjes bij de ontvangst tellen', 'een vroege lesronde', 'een late lesronde', 'Het ondiepe bad heeft blauwe tegels', 'in de lade naast de handdoeken'),
        ('de looprichting in een streekmuseum', 'de pijlen langs de trap opnieuw tekenen', 'de rubberstrips bij de deur vastzetten', 'één richting langs de zalen', 'twee richtingen langs de zalen', 'De trap heeft een brede tussenlanding', 'achter de vitrine met de klok'),
        ('de sensoren van een watermeetproject', 'de meetpunten langs het kanaal inmeten', 'de aansluitingen aan de peilstokken controleren', 'een wekelijkse aflezing', 'een dagelijkse aflezing', 'De meetpaal heeft een oranje bovenkant', 'bij de westelijke sluisdeur'),
        ('de sorteerplek van een afvalcoöperatie', 'de bakken voor glas verplaatsen', 'de wielen van de lage karren smeren', 'gescheiden aanrijstroken', 'gedeelde aanrijstroken', 'Het terrein is met betonnen platen bedekt', 'naast het hek met de brievenbus'),
        ('de ontvangst bij een atletiekwedstrijd', 'de startnummers per afstand tellen', 'de linten voor de finishzone oprollen', 'een brede startstrook', 'een smalle startstrook', 'De tribune heeft vijf rijen banken', 'onder de tafel bij de poort'),
        ('de toegankelijkheid van een leeskring', 'de deelnemerslijst voor de middag nalezen', 'de groteletterboeken naast de stoelen leggen', 'een vaste pauze halverwege', 'een pauze alleen op verzoek', 'De leeszaal heeft ramen op het zuiden', 'in de tas bij de deurmat'),
    ),
    'heldout': (
        ('een publieksavond in een sterrenwacht', 'de lensbescherming schoonmaken', 'de batterijen van de rode lampjes vervangen', 'kleine kijkgroepjes', 'één grote kijkgroep', 'De koepel heeft een smalle opening', 'onder de ronde telescooptafel'),
        ('de inrichting van een dierenambulance', 'de riemen in de ziekenkooi vervangen', 'de schuifsloten van de draagmand nakijken', 'een dubbele kooischeiding', 'een enkele kooischeiding', 'De laadruimte heeft een witte wandbekleding', 'achter de uitschuifbare vloerplaat'),
        ('de koude zone van een foodfestival', 'de koelwagen aansluiten', 'de rekken voor de bewaarbakken reinigen', 'aparte koelvakken per kraam', 'gedeelde koelvakken voor alle kramen', 'De vrachtzone heeft twee toegangspoorten', 'aan de achterkant van de watertank'),
        ('de ochtenduitzending van een wijkradio', 'de geluidsbestanden van het nieuws klaarzetten', 'de koptelefoons in de spreekcel testen', 'een vaste volgorde van rubrieken', 'een vrije volgorde van rubrieken', 'De spreekcel heeft een schuine glaswand', 'in de ondiepe vakkenkast'),
        ('de jeugdronde van een schaakclub', 'de wedstrijdklokken gelijkzetten', 'de reserveborden per tafel verdelen', 'korte speelronden', 'lange speelronden', 'De speelruimte heeft een houten vloer', 'naast het bord met de uitslagen'),
        ('de toegang tot een droogdok', 'de afzetting bij het dok nalopen', 'de lampen aan de vluchtroute controleren', 'een doorgang aan stuurboordzijde', 'een doorgang aan bakboordzijde', 'De dokrand heeft een gele veiligheidsstrook', 'achter de betonnen trap'),
        ('een lentebezoek aan een beeldentuin', 'de toegangspolsen per groep tellen', 'de zachte doppen onder de banken vastmaken', 'een lus langs de buitenrand', 'een lus door het middendeel', 'Het grasveld loopt licht af naar de vijver', 'in de kast bij het zijhek'),
        ('de verpakking van een imkerij', 'de deksels van de glazen potten controleren', 'de dozen voor de honingoogst vouwen', 'kleine potmaten', 'grote potmaten', 'De werkruimte heeft een tegelplint', 'boven de rekken met lege kratten'),
        ('de veilige hoek van een dansstudio', 'de randen van de oefenmatten vastplakken', 'de gripstroken voor de barre schoonmaken', 'een ruime afstand tussen de matten', 'een korte afstand tussen de matten', 'De spiegelwand bestaat uit drie delen', 'achter de lage schuimwand'),
        ('de proefbalie van een koffiebranderij', 'de weegschaaltjes op nul zetten', 'de handmolens bij de proefstoelen afstellen', 'een lichte branding als standaard', 'een donkere branding als standaard', 'Het plafond bevat zichtbare houten balken', 'onder de tafel met het zwarte blad'),
        ('het laden van een kanoaanhanger', 'de spanbanden om de bovenste dragers leggen', 'de peddelriemen per boot uitzoeken', 'een zijwaartse laadvolgorde', 'een laadvolgorde vanaf de achterkant', 'De aanhanger heeft twee lage wielkasten', 'in de bak onder de achterste drager'),
        ('de opslag van een archeologieteam', 'de borsteltjes bij de vondstbakken leggen', 'de zachte kussens in de transportkisten plaatsen', 'lage stapels per werkvak', 'hoge stapels per werkvak', 'De opslagruimte heeft stenen gewelven', 'bij de smalle deur van de kelder'),
    ),
}

# Each split has independently authored event wording as well as different facts.
# In particular, held-out text is not a renamed calibration sentence bank.
WORDINGS = {
    'development': (
        'Ik ga {work}. Je kunt erop rekenen dat ik dat morgen uitvoer.',
        'Over {scope} zijn we het eens geworden: we gaan uit van {choice}. De keuze is gemaakt.',
        'We weten bij {scope} nog niet hoeveel hulp er nodig is. Dat is een open informatiepunt.',
        'We draaien het eerdere besluit voor {scope} om: {old} wordt vervangen door {choice}. Dat is nu onze keuze.',
        'Een werkidee dat ik overweeg: {work}. Het is alleen een mogelijkheid, nog geen belofte.',
        'Bij {scope} is geen enkel besluit genomen. Er is bewust niets gekozen.',
        'Mijn uitroep "Wie wil er extra chaos?" was retorisch. Ik verwacht daar geen antwoord op.',
        'Mijn eerdere toezegging betrof dit werk: {work}. Ik trek die toezegging nu in.',
        '{fact}. Dat is alleen een beschrijving van de ruimte.',
        'Dit werk zeg ik vast toe: {work}. Bij {scope} ontbreekt nog het antwoord hoeveel hulp er nodig is.',
        'Voor {scope} spreken we voorlopig een voorkeur voor {choice} uit. Die keuze is nog niet definitief.',
        'Voor {scope} bestaat geen taak. Er wordt van niemand opvolgwerk gevraagd.',
        'Voor {scope} ligt de keuze vast op {choice}. Daarnaast staat mijn taak vast: {work}.',
        'De plaats van het materiaal was eerst onbekend, maar is opgehelderd: het staat {place}. Die vraag is opgelost.',
        'Mijn zekere toezegging is dit werk: {work}. {other} is slechts een werkidee waarvoor ik niets heb toegezegd.',
        'Bij {scope} missen we nog een tijdstip. Wanneer het beschikbaar komt, weten we niet.',
    ),
    'calibration': (
        'Reken op mij voor dit werk: {work}. Het staat voor morgen ingepland en ik voer het uit.',
        'De knoop is doorgehakt. Voor {scope} houden we {choice} aan; zo is het nu vastgesteld.',
        'Eén gegeven ontbreekt nog bij {scope}: hoeveel hulpmiddelen zijn nodig? Daar is geen antwoord op gekomen.',
        'Onze vorige uitkomst voor {scope} houden we niet aan. Het vastgelegde besluit verandert van {old} naar {choice}.',
        'Als klus kan ik misschien {work}. Ik draag dat nu aan als werkidee en verbind me er nog niet aan.',
        'Er is rond {scope} vandaag juist geen knoop doorgehakt. Een genomen besluit ontbreekt.',
        '"Wie zit er nu op wanorde te wachten?" was alleen een retorische opmerking. Het is geen open informatiepunt.',
        'Eerder beloofd werk: {work}. Die toezegging vervalt nu: ik herroep het eerder beloofde werk.',
        '{fact}. Meer dan deze feitelijke omschrijving meld ik hier niet.',
        'Mijn vaste taak is {work}. Hoeveel hulpmiddelen bij {scope} nodig zijn, blijft daarnaast een onbeantwoorde vraag.',
        'We hebben ons voorlopig uitgesproken voor {choice} bij {scope}. Het gaat om een keuze die nog niet is bekrachtigd.',
        'Aan {scope} hangt uitdrukkelijk geen opdracht. Niemand heeft hier werk te verrichten.',
        'De vastgestelde uitkomst voor {scope} is {choice}. Mijn vaste taak daarbij: {work}.',
        'De vraag waar het materiaal staat, is al afgehandeld: {place}. Het antwoord is dus bekend.',
        'Vast toegezegd werk: {work}. Alleen voorgesteld werk: {other}; daarvoor heb ik niets toegezegd.',
        'De beschikbaarheid van {scope} levert nog een informatiegat op: wanneer kunnen we erbij? Dat weten we nog niet.',
    ),
    'heldout': (
        'Zet mij als uitvoerder bij {work}; donderdag lever ik dat af. Dat is mijn vaste afspraak.',
        'Als uitkomst van dit overleg staat voor {scope} vast: {choice}. Dat besluit is rond.',
        'Bij {scope} blijft de benodigde hoeveelheid hulp onbekend. Op de vraag hoeveel mensen nodig zijn, hebben we nog geen uitsluitsel.',
        'Het oude besluit voor {scope} wordt door een ander vervangen. We laten {old} achter ons en leggen nu {choice} vast.',
        'Een mogelijk werkidee is: {work}. Ik opper het alleen; een toezegging is er nog niet.',
        'Een vastgelegde uitkomst is er voor {scope} niet. Niemand heeft op dit punt een keuze gemaakt.',
        'Ik zei "Moet het soms nog rommeliger?" als retorische verzuchting. Daar ligt geen vraag achter die nog beantwoord moet worden.',
        'Eerder had ik dit werk op me genomen: {work}. Die toezegging heb ik nu ingetrokken.',
        'Alleen een constatering ter verduidelijking: {fact_lower}. Dit beschrijft de bestaande ruimte.',
        'Ik zeg dit werk vast toe: {work}. Voor {scope} is nog steeds onduidelijk hoeveel mensen nodig zijn; die vraag staat open.',
        'Voor {scope} is {choice} onze voorlopige uitkomst geworden. We behandelen die keuze nog niet als definitief.',
        'Er is voor {scope} geen werk aan iemand opgedragen. Opvolgtaken zijn hier uitdrukkelijk niet aan de orde.',
        'We komen voor {scope} definitief uit op {choice}. Ernaast ligt mijn vaste taak vast: {work}.',
        'Voor het materiaal weten we de locatie inmiddels: {place}. De eerdere vraag daarover is beantwoord.',
        'Bij dit onderdeel zijn twee werkstatussen genoemd: {work} zeg ik zeker toe; {other} stel ik hoogstens als idee voor.',
        'Wat bij {scope} nog niet is opgehelderd, is het moment van toegang. Op de vraag wanneer dat kan, ontbreekt een antwoord.',
    ),
}
_REFERENCES = ('Berk', 'Ceder', 'Den', 'Es', 'Iep', 'Jenever', 'Kastanje', 'Linde', 'Meidoorn', 'Notelaar', 'Olm', 'Plataan')


def fixture(split, family, variant, length=None, suffix=''):
    if split not in FAMILIES or not 0 <= family < 12 or not 0 <= variant < 8:
        raise ValueError('unknown_managed_authored_fixture')
    length = 8 + variant if length is None else length
    if isinstance(length, bool) or not isinstance(length, int) or not 1 <= length <= 512:
        raise ValueError('bounded_authored_length_required')
    identifier = f'nl-managed-v1-{split}-f{family:02d}-v{variant}{suffix}'
    topic, task, other_task, choice, old, fact, place = FAMILIES[split][family]
    phase = {'development': 2, 'calibration': 5, 'heldout': 11}[split]
    utterances, answers, gold = [], [], []
    for index in range(length):
        case = (index + family * 5 + variant * 7 + phase) % len(CASE_STATES)
        reference = f'{_REFERENCES[family]}-{variant + 1}-{index + 1:03d}'
        scope = f'onderdeel {reference}'
        work = f'{task} voor {scope}'
        other = f'{other_task} voor {scope}'
        sentence = WORDINGS[split][case].format(work=work, other=other, scope=scope, choice=choice,
            old=old, fact=fact, fact_lower=fact[0].lower() + fact[1:], place=place)
        text = f'{topic[0].upper() + topic[1:]} / {reference}: {sentence}'
        if variant == 3:
            text = 'Nou, ' + text[0].lower() + text[1:]
        elif variant == 7:
            text = text.replace(';', ',')
        labels = {kind: CASE_STATES[case].get(kind, 'none') for kind in KINDS}
        labels.update({kind + '_support': CASE_SUPPORT[case][kind] for kind in KINDS})
        utterances.append({'start_ms': index * 5000, 'end_ms': index * 5000 + 4000, 'speaker': 'others',
                           'text': text, 'failed': False, 'voice': None})
        answers.append(labels)
        gold.extend({'window_index': index, 'kind': kind, 'commitment': labels[kind], 'text': text,
                     'owner': None, 'due': None}
                    for kind in KINDS if labels[kind + '_support'] == 'supported' and labels[kind] not in ('none', 'negated'))
    return {'id': identifier, 'family_id': f'nl-managed-v1-{split}-family-{family:02d}',
            'split': split, 'language': 'nl', 'utterances': utterances}, answers, gold


def experiment(configuration=None, expected_actual=None):
    if (configuration is None) != (expected_actual is None):
        raise ValueError('supply_both_route_configuration_and_expected_identity')
    panel, panel_answers, panel_gold = fixture('development', 0, 0, 16, '-resource-panel')
    stress, stress_answers, stress_gold = fixture('development', 1, 1, 512, '-long-stress')
    fixtures = [panel, stress]
    expected = {panel['id']: panel_answers, stress['id']: stress_answers}
    gold = {panel['id']: panel_gold, stress['id']: stress_gold}
    for split in FAMILIES:
        for family in range(12):
            for variant in range(8):
                value, answers, items = fixture(split, family, variant)
                fixtures.append(value); expected[value['id']] = answers; gold[value['id']] = items
    value = {'schema_version': 3, 'corpus_id': 'authored-managed-decider-meetings-v1',
        'provenance': {'source': 'Fresh authored Dutch fictional episodes and independent state/kind-support judgments; no model output supplies gold.',
            'real_meeting_content': False, 'natural_asr': False, 'historical_quality_credit': False,
            'split_unit': 'fictional episode family, including every related variant and length',
            'wording': 'Independent development, calibration and held-out wording banks and topic families; no episode crosses splits.',
            'source_scope': 'SOURCE 0 judgments; adjacent sources belong to different explicitly numbered episode parts.',
            'details': 'Verbatim passage cards only; owner, due and prior-link extraction have no authored credit.',
            'context_coverage': 'All 8..15 lengths and 12 calibration families are authored; actual production export slot/batch/question coverage is required before inference. Authored lengths alone do not prove 288 contexts.'},
        'fixtures': fixtures, 'expected_answers': expected, 'gold_items': gold,
        'resource_fixture_id': panel['id'], 'stress_fixture_id': stress['id']}
    if configuration is not None:
        value['configuration'] = copy.deepcopy(configuration)
        value['expected_actual'] = copy.deepcopy(expected_actual)
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    path = Path(args.output)
    if path.exists():
        raise ValueError('preserve_existing_authored_spec')
    path.parent.mkdir(parents=True, exist_ok=True)
    value = experiment()
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'fixtures': len(value['fixtures']), 'production_export_coverage': 'required_before_inference'}))


if __name__ == '__main__':
    main()
