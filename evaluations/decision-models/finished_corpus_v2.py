"""Authored invented Dutch passage judgments, independent of model or decoder output."""

from __future__ import annotations

from collections import Counter, defaultdict
import json
from pathlib import Path

KINDS = ('action', 'decision', 'open_question', 'changed_commitment')
STATES = ('none', 'tentative', 'confirmed', 'retracted', 'unresolved', 'changed', 'unknown', 'negated')
SITES = {
    'development': ('de havenkiosk', 'het atelier aan de beek', 'de bibliotheekzaal', 'de oude drukkerij',
                    'het buurthuis', 'de koffiebranderij', 'de schooltuin', 'het fietsenpunt',
                    'de markthal', 'het tuinpaviljoen', 'de stadswerkplaats', 'de muziekschool'),
    'calibration': ('het station bij de heuvel', 'de bakkerij op het plein', 'de gemeenteloods', 'de zwemclub',
                    'het archiefdepot', 'de museumwinkel', 'het jeugdcentrum', 'de wijkgarage',
                    'het zorgatelier', 'het natuurbezoekerscentrum', 'de kaaswinkel', 'de toneelvereniging'),
    'heldout': ('het veerhuis', 'de dierenopvang', 'de filmzaal', 'de technische balie',
                'de groentewinkel', 'de dorpsboekerij', 'de sportloods', 'de keramiekstudio',
                'het kindercentrum', 'de ontvangsthal', 'de wandelvereniging', 'het café aan het kanaal'),
}
CALIBRATION_B_SITES = ('de watertoren', 'de pakjesbalie', 'het winterpaviljoen', 'de glasblazerij',
    'het servicepunt aan de sluis', 'de buurtkeuken', 'het boekenmagazijn', 'de sterrenwacht',
    'de speelotheek', 'de kruidenwinkel', 'de wijkkliniek', 'de onthaalpost')
NEUTER_OBJECTS = {'reserveringsblad', 'inventarisblad', 'rooster', 'afsprakenblad', 'opbouwplan'}
OBJECTS = ('controlekaart', 'voorraadlijst', 'toegangspas', 'leveringsmap', 'reserveringsblad', 'onderhoudsfiche',
           'routekaart', 'weekplanning', 'instructiemap', 'bestelkaart', 'inventarisblad', 'rooster',
           'veiligheidsfiche', 'prijslijst', 'ontwerptekening', 'afsprakenblad', 'reparatiekaart',
           'bezoekerslijst', 'kostenstaat', 'werklijst', 'verzendfiche', 'uitleenkaart', 'meetstaat', 'opbouwplan')
# Labels follow authored SOURCE 0 meaning. Neighbouring turns are never label evidence.
CASE_LABELS = (
    {'action': 'confirmed'},
    {'decision': 'confirmed'},
    {'open_question': 'unresolved'},
    {'decision': 'changed', 'changed_commitment': 'changed'},
    {'action': 'tentative'},
    {'decision': 'negated'},
    {},
    {'action': 'retracted', 'changed_commitment': 'retracted'},
    {},
    {'action': 'confirmed', 'open_question': 'unresolved'},
    {'decision': 'tentative'},
    {'action': 'negated'},
    {'decision': 'confirmed', 'action': 'confirmed'},
    {},
    {'action': 'unknown'},
    {'open_question': 'unresolved'},
)


def _sentence(case, item, other, site, style):
    """Three independently worded variants per semantic case; no inferred labels."""
    variants = (
        (f'Ik zal {item} morgen controleren en dat werk zelf afmaken.',
         f'Voor {item} neem ik de controle op mij. Ik lever die controle morgen op.',
         f'{item.capitalize()} kijk ik na; dat is mijn concrete taak voor morgen.'),
        (f'Het definitieve besluit is dat we voor {item} de blauwe uitvoering gebruiken.',
         f'We hebben de keuze voor {item} gemaakt: het wordt definitief de blauwe versie.',
         f'Voor {item} ligt ons besluit vast. We kiezen de blauwe variant.'),
        (f'Wie kan {item} volgende week beheren? Op die vraag heeft nog niemand een antwoord.',
         f'Het blijft een open vraag wie {item} volgende week beheert. Niemand weet dat nu.',
         f'Wie wordt de beheerder van {item}? Dat punt is nog onbeantwoord.'),
        (f'Ons eerdere besluit over {item} wordt gewijzigd: in plaats van de rode versie kiezen we nu de groene.',
         f'We vervangen onze vorige keuze voor {item}. Het besluit verandert van rood naar groen.',
         f'De eerdere afspraak over {item} passen we aan; de vastgelegde keuze is voortaan groen in plaats van rood.'),
        (f'Ik zou {item} eventueel kunnen voorbereiden, maar het is nog maar een voorstel en geen vaste toezegging.',
         f'Misschien bereid ik {item} voor. Ik overweeg die taak, maar heb ze nog niet toegezegd.',
         f'Als voorstel kan ik {item} alvast klaarzetten; dat is op dit moment slechts een mogelijke taak.'),
        (f'Over {item} is uitdrukkelijk geen besluit genomen.',
         f'We hebben vandaag geen keuze of beslissing over {item} gemaakt.',
         f'Er ligt geen beslissing vast voor {item}; daar is geen besluit over genomen.'),
        (f'In het opleidingsvoorbeeld staat letterlijk: "Wie controleert {item}?" Dit is een geciteerd oefenvoorbeeld, geen vraag voor deze vergadering.',
         f'De lesfiche bevat de voorbeeldvraag "Waar ligt {item}?" We lezen alleen dat fictieve voorbeeld voor.',
         f'Als voorbeeldzin in de handleiding staat "Wie neemt {item} mee?" Het gaat om een citaat uit een oefening.'),
        (f'Ik trek mijn eerdere toezegging om {item} te controleren hierbij in. Die beloofde taak gaat niet door.',
         f'Mijn eerdere belofte voor de controle van {item} vervalt. Ik trek die concrete toezegging terug.',
         f'De controle van {item} had ik toegezegd, maar die toezegging herroep ik nu.'),
        (f'{item.capitalize()} ligt op een plank in {site}. Dit is alleen achtergrondinformatie.',
         f'Voor de achtergrond: {item} heeft een grijze kaft en ligt al in {site}.',
         f'{item.capitalize()} is een bestaand document uit {site}; ik beschrijf alleen waar het ligt.'),
        (f'Ik werk {item} morgen bij. Wie zal {other} beheren? Die laatste vraag is nog open.',
         f'Het bijwerken van {item} doe ik morgen zelf. Voor {other} weten we nog niet wie de beheerder wordt.',
         f'Mijn concrete taak is {item} morgen aanpassen. De vraag wie {other} beheert is nog niet beantwoord.'),
        (f'Ons voorlopige besluit voor {item} is de kleine uitvoering; dit besluit is nog niet definitief bevestigd.',
         f'Voor {item} hebben we een tentatieve keuze: de kleine variant. De definitieve bevestiging ontbreekt nog.',
         f'Voorlopig kiezen we bij {item} de kleine versie. Dat is een voorlopig besluit, nog geen definitieve keuze.'),
        (f'Niemand hoeft {item} te controleren; voor dit document is uitdrukkelijk geen actie nodig.',
         f'Er is geen opdracht om {item} na te kijken. Dat werk wordt van niemand gevraagd.',
         f'Voor {item} bestaat geen taak of opvolgactie. Niemand moet hiervoor iets uitvoeren.'),
        (f'Het definitieve besluit is dat {item} digitaal wordt bewaard. Ik zal het document daarom morgen inscannen.',
         f'We kiezen definitief voor digitale opslag van {item}. Het inscannen neem ik morgen op mij.',
         f'Voor {item} ligt de keuze voor digitale bewaring vast; mijn concrete taak is het document morgen te scannen.'),
        (f'De vraag waar {item} ligt is al beantwoord: het ligt in {site}. Daarover staat niets meer open.',
         f'Waar {item} ligt weten we intussen: in {site}. Die vraag is opgelost.',
         f'Het antwoord op de eerdere vraag naar {item} is gegeven: het ligt in {site}. Dit punt is afgehandeld.'),
        (f'Ik controleer {item} zeker morgen; de controle van {other} overweeg ik alleen nog en is niet toegezegd.',
         f'Voor {item} heb ik de controle morgen vast toegezegd. Het controleren van {other} is nog slechts een mogelijke taak.',
         f'Mijn controle van {item} staat vast voor morgen, maar een controle van {other} is alleen een tentatief voorstel.'),
        (f'Hoe raakt {item} volgende week ter plaatse? Niemand kent het antwoord en de vraag blijft open.',
         f'De vraag hoe we {item} ter plaatse krijgen is nog onbeantwoord. We weten dat nog niet.',
         f'Hoe wordt {item} volgende week aangeleverd? Dat weten we nog niet; dit is een open vraag.'),
    )
    return variants[case][style % 3]


def _sentence_b(case, item, other, site, style):
    """Fresh calibration wording, authored without inspecting attempt-A scores."""
    variants = (
        (f'Het nakijken van {item} is mijn taak. Ik rond dat morgen af.',
         f'Voor morgen staat mijn concrete opdracht vast: {item} controleren.',
         f'Ik beloof dat ik morgen {item} volledig zal nalopen.'),
        (f'We leggen de keuze voor {item} nu definitief vast op de zilveren variant.',
         f'Er is een definitieve beslissing voor {item}: we nemen de zilveren uitvoering.',
         f'De knoop voor {item} is doorgehakt; de zilveren versie is gekozen.'),
        (f'Er is nog geen antwoord op de vraag wie {item} zal beheren.',
         f'Wie neemt het beheer van {item} op zich? Die vraag blijft onbeantwoord.',
         f'Het is een open vraag wie verantwoordelijk wordt voor het beheer van {item}.'),
        (f'Onze afspraak over {item} verandert: de eerder gekozen brede versie wordt de smalle versie.',
         f'Het vorige besluit over {item} passen we aan van breed naar smal.',
         f'Wij herzien de bestaande keuze voor {item}: we vervangen breed door smal.'),
        (f'Ik zou eventueel {item} kunnen nakijken. Dat is slechts een mogelijke taak, geen vaste belofte.',
         f'Het voorbereiden van {item} kan misschien voor mij zijn, maar ik heb dat nog niet toegezegd.',
         f'Als tentatief voorstel wil ik {item} klaarleggen; daar is nog geen toezegging voor.'),
        (f'Voor {item} hebben we geen besluit vastgelegd; er is nog geen keuze gemaakt.',
         f'Er is uitdrukkelijk geen beslissing genomen over {item}.',
         f'Een beslissing over {item} ontbreekt; er is geen besluit genomen.'),
        (f'Het cursusblad geeft alleen als fictief voorbeeld de zin "Wie onderhoudt {item}?" We citeren een oefenzin.',
         f'De vraag "Wie pakt {item}?" staat in een lesvoorbeeld; niemand stelt die hier aan de groep.',
         f'Dit is een voorgelezen oefenvoorbeeld: "Hoe controleren we {item}?" Het is uitsluitend een citaat.'),
        (f'De controle van {item} was door mij toegezegd. Ik trek die toezegging nu terug.',
         f'Ik herroep de belofte dat ik {item} zou controleren.',
         f'Mijn bestaande toezegging voor {item} wordt hierbij ingetrokken; die beloofde taak voer ik niet uit.'),
        (f'Ter situering: {item} heeft drie bladen en komt uit {site}.',
         f'Dit is slechts een beschrijving: {item} heeft een matte omslag.',
         f'Voor de achtergrond zeg ik alleen dat {item} in {site} ligt.'),
        (f'Ik neem {item} morgen helemaal door. Hoe het beheer van {other} wordt geregeld, is nog een open vraag.',
         f'Mijn taak is om {item} morgen bij te werken; wie {other} beheert is nog onbekend.',
         f'Ik zal {item} morgen afwerken. Wie beheert {other}? Dat antwoord heeft nog niemand.'),
        (f'Ons besluit over {item} is nog voorlopig: we houden het op de mediumvariant, zonder definitieve bekrachtiging.',
         f'De tentatieve keuze voor {item} is de mediumversie; bevestigd is dat besluit nog niet.',
         f'Voor {item} is de middelgrote versie slechts voorlopig gekozen; de beslissing is nog tentatief.'),
        (f'Er wordt van niemand gevraagd om {item} te controleren. Daar geldt geen actie.',
         f'{item.capitalize()} hoeft door niemand gecontroleerd te worden; er is geen taak van dit punt gemaakt.',
         f'Voor {item} is uitdrukkelijk geen taak voor iemand: niemand moet hier iets doen.'),
        (f'Voor {item} besluiten we definitief digitaal te bewaren. Het digitaliseren doe ik morgen.',
         f'Onze keuze voor {item} is vastgelegde digitale opslag; ik zal daarom morgen het scanwerk uitvoeren.',
         f'Het vaststaande besluit is dat {item} digitaal wordt opgeslagen. Ik neem morgen het scannen van het document op mij.'),
        (f'De locatievraag over {item} is afgehandeld: het ligt in {site}. Dat is inmiddels duidelijk.',
         f'We weten nu waar {item} ligt, namelijk in {site}. De voorgaande vraag is beantwoord.',
         f'Het antwoord voor de ligging van {item} is reeds gegeven; dat punt is niet meer open.'),
        (f'Ik zeg de controle van {item} voor morgen vast toe; {other} zou ik misschien ook kunnen nakijken.',
         f'Bij {item} heb ik de taak voor morgen beloofd; bij {other} denk ik alleen nog aan een mogelijke taak.',
         f'{item.capitalize()} controleer ik morgen zeker, {other} alleen misschien en nog zonder toezegging.'),
        (f'Het is onbeantwoord hoe {item} volgende week op zijn bestemming komt. Dat blijft een open vraag.',
         f'Hoe gaan we {item} ter plaatse krijgen? Er is nog geen antwoord op die vraag.',
         f'Hoe {item} wordt aangeleverd is tot nu toe een onopgeloste vraag.'),
    )
    return variants[case][style % 3]


def _utterance(split, family, variant, position, attempt='A'):
    # Multiplication by five rotates each batch slot through twelve distinct authored
    # cases; even sparse changed-commitment evidence stays represented in every context.
    phase = {'development': 0, 'calibration': 3, 'heldout': 9}[split]
    case = (position + family * 5 + variant * 7 + phase) % len(CASE_LABELS)
    fresh = split == 'calibration' and attempt == 'B'
    site = CALIBRATION_B_SITES[family] if fresh else SITES[split][family]
    primary = OBJECTS[(position + variant * 3) % len(OBJECTS)]
    secondary = OBJECTS[(position + variant * 3 + 11) % len(OBJECTS)]
    tag = f'{chr(65 + variant)}{position + 1:02d}'
    article = lambda noun: 'het' if fresh and noun in NEUTER_OBJECTS else 'de'
    item, other = f'{article(primary)} {primary} {tag} voor {site}', f'{article(secondary)} {secondary} {tag} voor {site}'
    text = (_sentence_b if fresh else _sentence)(case, item, other, site, family + variant + position + phase)
    # Authored text perturbations, not measured ASR. Crucial polarity words remain exact.
    if variant == 3:
        text = 'Euh, ' + text[0].lower() + text[1:]
    elif variant == 7:
        text = text.replace(';', ' ').replace('.', '').replace('Ik ', 'Ik, ik ', 1)
    labels = {kind: CASE_LABELS[case].get(kind, 'none') for kind in KINDS}
    return text, labels, case


def fixture(split, family, variant, length=None, suffix='', attempt='A'):
    length = 8 + variant if length is None else length
    namespace = f'{split}-b' if split == 'calibration' and attempt == 'B' else split
    identifier = f'nl-{namespace}-f{family:02d}-v{variant}{suffix}'
    rows, answers, gold, cases = [], [], [], []
    for index in range(length):
        text, labels, case = _utterance(split, family, variant, index, attempt)
        rows.append({'start_ms': index * 5000, 'end_ms': index * 5000 + 4000, 'speaker': 'others', 'text': text})
        answers.append(labels)
        cases.append(case)
        gold.extend({'window_index': index, 'kind': kind, 'commitment': label, 'text': text, 'owner': None, 'due': None}
                    for kind, label in labels.items() if label not in ('none', 'negated'))
    return {'id': identifier, 'family_id': f'nl-{namespace}-family-{family:02d}', 'split': split, 'language': 'nl',
            'utterances': rows}, answers, gold, cases


def experiment(attempt='B'):
    if attempt not in ('A', 'B'): raise ValueError('unknown_authored_attempt')
    panel, panel_answers, panel_gold, _ = fixture('development', 0, 0, 16, '-resource-panel')
    stress, stress_answers, stress_gold, _ = fixture('development', 1, 1, 512, '-long-stress')
    fixtures, answers, gold = [panel, stress], {panel['id']: panel_answers, stress['id']: stress_answers}, {panel['id']: panel_gold, stress['id']: stress_gold}
    for split in ('development', 'calibration', 'heldout'):
        for family in range(12):
            for variant in range(8):
                item, labels, judgments, _ = fixture(split, family, variant, attempt=attempt)
                fixtures.append(item)
                answers[item['id']], gold[item['id']] = labels, judgments
    return {'schema_version': 2, 'corpus_id': 'authored-dutch-finished-passages-v2' if attempt == 'A' else 'authored-dutch-finished-passages-v2-attempt-b',
        'provenance': {'source': 'New independently authored Dutch text and per-SOURCE0 semantic judgments; no model output or production decoder supplies labels.',
                       'natural_asr': False, 'real_meeting_content': False,
                       'split_unit': 'family_including_all_lengths_and_text_variants',
                       'scope': 'Source-quoted passage cards; exact owner, due and prior-link extraction unsupported and null.',
                       'quality_repeats': 'No exact utterance text reused between splits; timing repeats add no semantic quality credit.'},
        'configuration': {'protocol': 'open_router_decisions', 'endpoint': 'https://openrouter.ai/api/alpha/decisions',
                          'model': 'perplexity/pplx-decider-v1.2', 'expected_model': 'perplexity/pplx-decider-v1.2-20261016',
                          'upstream': 'perplexity', 'max_input_bytes': 65536, 'timeout_secs': 90},
        'expected_actual': {'model': 'perplexity/pplx-decider-v1.2-20261016', 'provider': 'Perplexity', 'revision': '',
                            'artifact_sha256': '', 'identity_evidence': 'provider_reported_version'},
        'fixtures': fixtures, 'expected_answers': answers, 'gold_items': gold,
        'resource_fixture_id': panel['id'], 'stress_fixture_id': stress['id']}


def context_coverage(spec):
    groups = defaultdict(list)
    for item in spec['fixtures']:
        if item['split'] != 'calibration': continue
        labels = spec['expected_answers'][item['id']]
        for index, row in enumerate(labels):
            size = min(8, len(labels) - (index // 8) * 8)
            for kind in KINDS:
                groups[size, index % 8, kind].append((row[kind], item['family_id']))
    return [{'batch_size': size, 'slot': slot, 'kind': kind, 'observations': len(rows),
             'classes': len({label for label, _ in rows}), 'families': len({family for _, family in rows})}
            for (size, slot, kind), rows in sorted(groups.items())]


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True)
    parser.add_argument('--attempt', choices=('A','B'), default='B')
    args = parser.parse_args()
    value = experiment(args.attempt)
    coverage = context_coverage(value)
    if any(row['observations'] < 10 or row['classes'] < 2 or row['families'] < 2 for row in coverage):
        raise ValueError('insufficient_authored_calibration_contexts')
    output = Path(args.output)
    if output.exists(): raise ValueError('preserve_existing_corpus')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    counts = Counter(item['split'] for item in value['fixtures'])
    print(json.dumps({'fixtures': dict(counts), 'contexts': len(coverage), 'minimum_context_observations': min(row['observations'] for row in coverage),
                      'model_requests': 0, 'real_content': False}))


if __name__ == '__main__': main()
