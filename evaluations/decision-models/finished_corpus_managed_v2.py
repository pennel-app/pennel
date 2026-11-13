"""Independent Dutch held-out episodes with exact V1 calibration source reuse.

This module authors sources and judgments only. It reads no predictions, fitted
locks, provider state or user content, and does not grant qualification.
"""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

import finished_corpus_managed_v1 as original

KINDS = original.KINDS
STATES = original.STATES
SUPPORT = original.SUPPORT
CASE_STATES = copy.deepcopy(original.CASE_STATES)
CASE_SUPPORT = copy.deepcopy(original.CASE_SUPPORT)

# Fictional topic, two possible tasks, selected/superseded alternatives, fact,
# and resolved location. All nine families are new and entirely held out.
FAMILIES = (
    ('de proefopstelling in een windtunnel',
     'de drukslangen op het meetblok aansluiten', 'de pitotbuizen boven het rooster uitlijnen',
     'een honingraatrooster', 'een gaasrooster',
     'De stroomlijnkap is matgrijs', 'links van het bedieningsluik'),
    ('de patroonwissel op een weefgetouw',
     'de scheringdraden door de hevels halen', 'de spoelen bij het aandrijfwiel sorteren',
     'een keperbinding', 'een platbinding',
     'De voorboom is van beukenhout', 'naast de remhendel'),
    ('de koele bewaarruimte van een filmarchief',
     'de zuurvrije hulzen rond de spoelen schuiven', 'de afdichtring van het luchtkanaal testen',
     'liggende opslag', 'staande opslag',
     'De metalen kast heeft geperforeerde zijkanten', 'onder de draaibare spoelhouder'),
    ('de vouwprocedure in een ballonwerkplaats',
     'de naden van het proefomhulsel nalopen', 'de ventielringen per maat rangschikken',
     'een dwarse vouw', 'een vouw in de lengterichting',
     'Het snijvlak is met vilt bekleed', 'tussen de rollen patroonfolie'),
    ('de vakindeling van een pakketkluis',
     'de tussenschotten in de onderste rij monteren', 'de scharnierpennen van de deurtjes invetten',
     'afwisselend diepe vakken', 'twee gelijke vakdieptes',
     'De kast heeft een zilverkleurige plint', 'in het vierkante dienstvak'),
    ('de warmteverdeling in een chocoladelaboratorium',
     'de spatels bij de tempereerbakken leggen', 'de voelers onder de smeltpot doormeten',
     'stapsgewijze verwarming', 'gelijkmatige verwarming',
     'Het marmeren koelblad is beige', 'rechts van de doseertrechter'),
    ('de proefpers van een penningenmakerij',
     'het stempelvlak ontvetten', 'de aanvoertrechter met proefschijfjes vullen',
     'een matte afwerking', 'een glanzende afwerking',
     'De pers staat op rubberblokken', 'achter het kantelbare voetstuk'),
    ('de verdampingstafel voor zoutkristallen',
     'de ondiepe schaaltjes op de houder zetten', 'de afsluitdopjes op de monsterbuizen vastdraaien',
     'een langzame droogstand', 'een snelle droogstand',
     'Het tafelblad is lichtblauw', 'binnen de ronde lekbak'),
    ('de oefentank van een onderwaterrobot',
     'de drijvers langs het frame vastmaken', 'de kabelgeleiders op scherpe randen nakijken',
     'een beweging langs de bodem', 'een beweging langs de wand',
     'De tankwand is doorzichtig', 'onder het verplaatsbare hijsrek'),
)

# Ordered by the existing CASE_STATES/CASE_SUPPORT semantics, including the
# single-label unknown action state when a firm and a proposed task coexist.
WORDINGS = (
    'Dit gaat gebeuren: {work}. Ik neem de uitvoering op me; de afspraak staat vast.',
    'Voor {scope} is de uitkomst {choice} aangenomen. Daarmee is het overlegpunt beslist.',
    'De aantallen voor {scope} ontbreken: hoeveel hulpmiddelen zijn vereist? Het antwoord moet nog komen.',
    'De vaststaande keuze voor {scope} wijzigt: {choice} komt in de plaats van {old}. De nieuwe afspraak geldt vanaf nu.',
    'Misschien wil ik {work}. Ik leg het als mogelijkheid neer, zonder me al tot uitvoering te verbinden.',
    'Een uitkomst over {scope} is niet vastgesteld. We hebben de mogelijkheden besproken, maar niets beslist.',
    'Ik citeer een spotzin uit een verzonnen toneeltekst: "Wie vraagt er om nóg meer gedoe?" Er wordt geen antwoord verwacht.',
    'Ik had {work} toegezegd. Bij deze vervalt mijn belofte; ik zal dat werk niet uitvoeren.',
    '{fact}. Dit is een toelichting op wat er al is.',
    'De uitvoering van {work} neem ik op me. Voor {scope} ontbreekt nog het aantal hulpmiddelen; die vraag blijft onbeantwoord.',
    'Als voorlopige uitkomst voor {scope} is {choice} genoteerd. Bekrachtiging van die keuze ontbreekt nog.',
    'Bij {scope} hoort geen uit te voeren klus. Er rust op niemand een werkafspraak.',
    'Voor {scope} is {choice} definitief aangenomen. Ik zorg aansluitend voor deze uitvoering: {work}.',
    'Over de vindplaats is geen onduidelijkheid meer: het materiaal ligt {place}. De eerder gestelde vraag is hiermee afgehandeld.',
    'Twee klussen voor {scope}: {work} doe ik zeker; {other} is enkel een suggestie waaraan ik me niet verbind.',
    'Voor {scope} is de toegangstijd onbekend. Wanneer is toegang mogelijk? Niemand heeft dat kunnen ophelderen.',
)
REFERENCES = ('Agaat', 'Basalt', 'Calciet', 'Dolomiet', 'Fluoriet', 'Gneis', 'Kwarts', 'Leisteen', 'Marmer')


def case_index(family, variant, index):
    return (index + family * 3 + variant * 5 + 1) % len(CASE_STATES)


def fixture(family, variant):
    if (type(family) is not int or type(variant) is not int
            or not 0 <= family < len(FAMILIES) or not 0 <= variant < 8):
        raise ValueError('unknown_managed_v2_authored_fixture')
    identifier = f'nl-managed-v2-heldout-f{family:02d}-v{variant}'
    topic, task, other_task, choice, old, fact, place = FAMILIES[family]
    utterances, answers, gold = [], [], []
    for index in range(8 + variant):
        case = case_index(family, variant, index)
        reference = f'{REFERENCES[family]}-{variant + 1}-{index + 1:03d}'
        sentence = WORDINGS[case].format(work=task, other=other_task, scope=f'proefvak {reference}',
                                       choice=choice, old=old, fact=fact, place=place)
        text = f'{topic[0].upper() + topic[1:]} / {reference}: {sentence}'
        labels = {kind: CASE_STATES[case].get(kind, 'none') for kind in KINDS}
        labels.update({kind + '_support': CASE_SUPPORT[case][kind] for kind in KINDS})
        utterances.append({'start_ms': index * 5000, 'end_ms': index * 5000 + 4000,
                           'speaker': 'others', 'text': text, 'failed': False, 'voice': None})
        answers.append(labels)
        gold.extend({'window_index': index, 'kind': kind, 'commitment': labels[kind], 'text': text,
                     'owner': None, 'due': None}
                    for kind in KINDS
                    if labels[kind + '_support'] == 'supported' and labels[kind] not in ('none', 'negated'))
    return {'id': identifier, 'family_id': f'nl-managed-v2-heldout-family-{family:02d}',
            'split': 'heldout', 'language': 'nl', 'utterances': utterances}, answers, gold


def experiment(configuration=None, expected_actual=None):
    if (configuration is None) != (expected_actual is None):
        raise ValueError('supply_both_route_configuration_and_expected_identity')
    previous = original.experiment()
    panel_ids = {previous['resource_fixture_id'], previous['stress_fixture_id']}
    fixtures = copy.deepcopy([value for value in previous['fixtures']
                              if value['split'] == 'calibration' or value['id'] in panel_ids])
    calibration_ids = [value['id'] for value in fixtures if value['split'] == 'calibration']
    expected = {value['id']: copy.deepcopy(previous['expected_answers'][value['id']]) for value in fixtures}
    gold = {value['id']: copy.deepcopy(previous['gold_items'][value['id']]) for value in fixtures}
    for family in range(len(FAMILIES)):
        for variant in range(8):
            value, answers, items = fixture(family, variant)
            fixtures.append(value)
            expected[value['id']] = answers
            gold[value['id']] = items
    value = {
        'schema_version': 3,
        'corpus_id': 'authored-managed-decider-meetings-v2',
        'provenance': {
            'source': 'Independently authored Dutch fictional episodes and literal kind/state/support gold; no model predictions or error examples were consulted.',
            'real_meeting_content': False,
            'natural_asr': False,
            'historical_quality_credit': False,
            'prior_link_extraction': False,
            'split_unit': 'fictional topic family, including every related variant and length',
            'wording': 'New held-out wording for all 16 existing semantics and nine new topic families; no new episode or wording template appears in V1.',
            'source_scope': 'SOURCE 0 judgments only; each mineral/variant/row reference names a separate fictional proefvak, including adjacent rows with the same topic.',
            'details': 'Verbatim passage cards only; owner, due and prior-link extraction have no authored credit.',
            'seen_sources': 'Only the unchanged V1 calibration and its 16-window resource and 512-window stress panels are retained; other seen V1 episodes are omitted.',
            'context_coverage': 'Exact calibration source reuse is descriptive provenance. Actual production export coverage and binding to the original fitted calibration evidence remain required before inference.',
        },
        'calibration_reuse': {
            'mode': 'exact_source_fixture_reuse',
            'source_corpus_id': previous['corpus_id'],
            'source_module': 'finished_corpus_managed_v1.py',
            'fixture_ids': calibration_ids,
            'fixture_count': len(calibration_ids),
        },
        'fixtures': fixtures,
        'expected_answers': expected,
        'gold_items': gold,
        'resource_fixture_id': previous['resource_fixture_id'],
        'stress_fixture_id': previous['stress_fixture_id'],
    }
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
    print(json.dumps({'fixtures': len(value['fixtures']), 'fresh_heldout_fixtures': 72,
                      'calibration_fixture_sources_reused': 96,
                      'production_export_coverage': 'required_before_inference'}))


if __name__ == '__main__':
    main()
