"""Fresh authored Dutch SOURCE-0 state and kind-support judgments for trial C.

No model output supplies labels. Fresh split wording/topics retain the historical
semantic scope; an explicit seen-B development regression receives no quality credit.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

KINDS = ('action', 'decision', 'open_question', 'changed_commitment')
STATES = ('none', 'tentative', 'confirmed', 'retracted', 'unresolved', 'changed', 'unknown', 'negated')
SUPPORT = ('supported', 'absent', 'unclear')
CASE_STATES = (
    {'action':'confirmed'}, {'decision':'confirmed'}, {'open_question':'unresolved'},
    {'decision':'changed','changed_commitment':'changed'}, {'action':'tentative'},
    {'decision':'negated'}, {}, {'action':'retracted','changed_commitment':'retracted'}, {},
    {'action':'confirmed','open_question':'unresolved'}, {'decision':'tentative'},
    {'action':'negated'}, {'decision':'confirmed','action':'confirmed'}, {},
    {'action':'unknown'}, {'open_question':'unresolved'},
)
# Presence is an independent authored judgment, including retracted/proposed
# events. An explicit denial of any task/choice is absent, not a supported card.
CASE_SUPPORT = (
    {'action':'supported'}, {'decision':'supported'}, {'open_question':'supported'},
    {'decision':'supported','changed_commitment':'supported'}, {'action':'supported'}, {}, {},
    {'action':'supported','changed_commitment':'supported'}, {},
    {'action':'supported','open_question':'supported'}, {'decision':'supported'}, {},
    {'decision':'supported','action':'supported'}, {}, {'action':'supported'}, {'open_question':'supported'},
)
CASE_SUPPORT = tuple({kind:row.get(kind,'absent') for kind in KINDS} for row in CASE_SUPPORT)

SITES = {
    'development': ('het herstelcafé','de uitleenpost','het scheepvaartdepot','de zaadbank',
        'de buurtbioscoop','de textielwerkplaats','het makerslokaal','de restauratiehal',
        'het dorpsloket','de landschapskring','het zangarchief','de buurtbakkerij'),
    'calibration': ('het naaiatelier','de materiaalkeuken','de taalstudio','het archieflokaal',
        'de watermolen','het afhaalpunt','de excursiebalie','het decoratelier',
        'de streekmarkt','de onderzoeksloods','het reisbureau','de houtzagerij'),
    'heldout': ('het panorama aan de dijk','de buurttent','het gereedschapshuis','de repetitiekelder',
        'het kijkcentrum','de spelwerkplaats','het verzamelhuis','de aanlegsteiger',
        'de expositieruimte','het keramiekmagazijn','de buurtbalie','het verzendatelier'),
}
OBJECTS = ('inschrijfformulier','uitnodiging','notulenbundel','transportschema','prijskaart','vergadermap',
    'toelatingsbrief','bezoekregeling','drukproef','werkprotocol','zaalfiche','koelplan','bouwschets',
    'informatiebrochure','materiaalstaat','voortgangstabel','evaluatieverslag','afleverschema',
    'verdeelstaat','trainingsplan','dienstrooster','verhuurlijst','begeleidboek','cursusblad')
NEUTER = {'inschrijfformulier','transportschema','werkprotocol','koelplan','evaluatieverslag',
          'afleverschema','trainingsplan','dienstrooster','begeleidboek','cursusblad'}


def _sentence(case, item, other, site, split):
    # Different authored wording is held out per split, not merely another topic.
    wordings = (
        (f'Ik neem het actualiseren van {item} voor mijn rekening; uiterlijk morgen is dat werk klaar.',
         f'Mijn opdracht voor morgen is het volledig bijwerken van {item}. Dat werk heb ik toegezegd.',
         f'Het aanvullen van {item} doe ik zelf. Ik beloof dat het morgen gereed is.'),
        (f'Onze definitieve keuze voor {item} valt op uitvoering B; die keuze staat vast.',
         f'Voor {item} is het besluit definitief: uitvoering B is geselecteerd.',
         f'Bij {item} geldt nu de vastgestelde keuze voor uitvoering B. Daarover is beslist.'),
        (f'Voor {item} ontbreekt nog een antwoord: wie houdt het bij? Dat is nog niet opgehelderd.',
         f'We missen nog de informatie wie {item} bijhoudt. Die vraag is niet opgelost.',
         f'Wie zal {item} bijhouden? Daarover staat nog een onbeantwoorde vraag op onze lijst.'),
        (f'De vastgelegde keuze voor {item} herzien we. Voortaan geldt uitvoering C, niet langer uitvoering A.',
         f'Het bestaande besluit voor {item} wordt aangepast van uitvoering A naar uitvoering C.',
         f'Voor {item} trekken we onze vroegere keuze voor uitvoering A recht: het nieuwe besluit is uitvoering C.'),
        (f'Een mogelijkheid is dat ik {item} alvast uitwerk. Ik doe daar nu alleen een voorstel voor.',
         f'Als mogelijke taak stel ik voor dat ik {item} uitwerk; dat is nog geen toezegging.',
         f'Ik denk erover om {item} uit te werken. Het gaat slechts om een voorgestelde taak.'),
        (f'We hebben de keuze voor {item} bewust niet gemaakt; er is op dit punt niets beslist.',
         f'Voor {item} ontbreekt een genomen besluit. We hebben uitdrukkelijk niets geselecteerd.',
         f'Er is vandaag geen keuze voor {item} vastgesteld; er is geen beslissing hierover genomen.'),
        (f'Op de oefenkaart lezen we de voorbeeldzin "Wie vult {item} aan?" Dat is een lescitaat, geen vraag die hier wordt gesteld.',
         f'Het opleidingsmateriaal toont "Waar bewaren we {item}?" als oefenvraag. Wij citeren alleen die fictieve leszin.',
         f'We lezen de zin "Wie haalt {item} op?" uitsluitend voor uit een demonstratie. Het is geen informatievraag in dit overleg.'),
        (f'Eerder had ik de bewerking van {item} beloofd. Op die belofte kom ik terug: ik voer die taak niet uit.',
         f'Mijn eerdere toezegging om {item} te bewerken trek ik in; die toegezegde opdracht vervalt.',
         f'Ik laat de eerder beloofde bewerking van {item} vallen. Ik herroep die toezegging.'),
        (f'Als toelichting: {item} bevat vier kolommen en een grijze omslag.',
         f'Alleen ter beschrijving: {item} is een map met een kartonnen rug uit {site}.',
         f'Ter informatie beschrijf ik {item}: het heeft een witte kaft en een genummerde inhoud.'),
        (f'Ik maak {item} morgen gereed. Voor {other} staat nog open wie het later zal bijhouden.',
         f'Het gereedmaken van {item} neem ik morgen op mij. Wie {other} zal bijhouden is nog onbeantwoord.',
         f'Mijn concrete werk is {item} morgen afronden. De vraag wie {other} bijhoudt blijft ondertussen open.'),
        (f'We neigen voor {item} voorlopig naar uitvoering D. De keuze is nog tentatief en niet bekrachtigd.',
         f'De keuze voor {item} blijft voorlopig op uitvoering D staan; het is nog geen definitief besluit.',
         f'Bij {item} hebben we een tijdelijke keuze voor uitvoering D vastgelegd. Die beslissing is nog tentatief.'),
        (f'Voor {item} wordt geen werk van iemand gevraagd. Er hoort op dit moment uitdrukkelijk geen taak bij.',
         f'Niemand krijgt een opdracht rond {item}; er is uitdrukkelijk geen opvolgwerk nodig.',
         f'Aan {item} koppelen we geen taak. Niemand moet hier werk voor uitvoeren.'),
        (f'We hebben definitief vastgelegd dat {item} naar het digitale archief gaat. Het inscannen verzorg ik morgen.',
         f'Ons vaststaande besluit voor {item} is digitaal archiveren. Ik neem het scanwerk morgen voor mijn rekening.',
         f'Voor {item} hebben we definitief digitale archivering gekozen. Mijn toegezegde taak is het morgen inscannen.'),
        (f'De eerder ontbrekende locatie van {item} is inmiddels bekend: het ligt in {site}. Die vraag is afgehandeld.',
         f'Inmiddels is de vraag naar de plaats van {item} opgelost. Het ligt in {site}, dus dit staat niet meer open.',
         f'Waar {item} ligt is al duidelijk geworden: in {site}. De vroegere vraag is beantwoord.'),
        (f'Het nakijken van {item} zeg ik vast toe voor morgen. Een bewerking van {other} is hoogstens een idee waarvoor ik nog niets heb toegezegd.',
         f'Voor {item} beloof ik de controle morgen; een bewerking van {other} stel ik alleen als mogelijke taak voor.',
         f'Mijn controle van {item} voor morgen is toegezegd. Het bewerken van {other} is daarnaast slechts een voorlopig voorstel.'),
        (f'Over het aanleveren van {item} missen we nog informatie: hoe komt het op tijd hier? Dat is nog onbeantwoord.',
         f'Er staat nog een informatievraag open over {item}: op welke manier wordt het hier aangeleverd?',
         f'Hoe {item} hier op tijd zal aankomen, is nog niet opgehelderd. Op die vraag hebben we geen antwoord.'),
    )
    return wordings[case][('development','calibration','heldout').index(split)]


def fixture(split, family, variant, length=None, suffix=''):
    if split not in SITES or not 0 <= family < 12 or not 0 <= variant < 8:
        raise ValueError('unknown_authored_fixture')
    length = 8+variant if length is None else length
    identifier = f'nl-c-{split}-f{family:02d}-v{variant}{suffix}'
    rows, answers, gold = [], [], []
    phase = {'development':0,'calibration':3,'heldout':9}[split]
    site = SITES[split][family]
    for position in range(length):
        case = (position+family*5+variant*7+phase) % 16
        tag = f'C{variant+1}-{position+1:02d}'
        names = [OBJECTS[(position+variant*3+offset) % len(OBJECTS)] for offset in (0,11)]
        item, other = [f'{"het" if noun in NEUTER else "de"} {noun} {tag} van {site}' for noun in names]
        text = _sentence(case,item,other,site,split)
        if variant == 3: text = 'Nou, '+text[0].lower()+text[1:]
        elif variant == 7: text = text.replace(';',' ').replace('.','')
        row = {kind:CASE_STATES[case].get(kind,'none') for kind in KINDS}
        row.update({kind+'_support':CASE_SUPPORT[case][kind] for kind in KINDS})
        rows.append({'start_ms':position*5000,'end_ms':position*5000+4000,'speaker':'others','text':text})
        answers.append(row)
        gold.extend({'window_index':position,'kind':kind,'commitment':row[kind],'text':text,'owner':None,'due':None}
                    for kind in KINDS if CASE_SUPPORT[case][kind]=='supported' and row[kind] not in ('none','negated'))
    return {'id':identifier,'family_id':f'nl-c-{split}-family-{family:02d}','split':split,'language':'nl','utterances':rows},answers,gold


def experiment():
    panel, panel_answers, panel_gold = fixture('development',0,0,16,'-resource-panel')
    stress, stress_answers, stress_gold = fixture('development',1,1,512,'-long-stress')
    regression = json.loads(Path(__file__).with_name('fixtures').joinpath('seen-b-semantic-regression-v3.json').read_text())
    assert regression['evidence_class']=='seen_development_regression' and regression['quality_credit'] is False
    seen = regression['fixture']
    fixtures = [panel,stress,seen]
    expected, gold = {panel['id']:panel_answers,stress['id']:stress_answers},{panel['id']:panel_gold,stress['id']:stress_gold}
    expected[seen['id']]=regression['expected_answers'];gold[seen['id']]=regression['gold_items']
    for split in ('development','calibration','heldout'):
        for family in range(12):
            for variant in range(8):
                value,answers,items = fixture(split,family,variant)
                fixtures.append(value);expected[value['id']]=answers;gold[value['id']]=items
    return {'schema_version':3,'corpus_id':'authored-dutch-supported-passages-c-v3',
        'provenance':{'source':'New authored Dutch source text and independent state/kind-support judgments; no model outputs or decoder generate labels.',
            'real_meeting_content':False,'natural_asr':False,'historical_quality_credit':False,
            'split_unit':'family including all lengths and text variants',
            'wording':'Three independent authored wordings per semantic case, held out by split.',
            'source_scope':'SOURCE 0 only; adjacent context supplies no event judgment.',
            'details':'Exact source-quoted passage cards only; owner, due and prior-link extraction unsupported.',
            'context_coverage':'Qualification requires actual production-export slot/batch/question coverage before inference; authored lengths alone do not prove it.'},
        'configuration':{'protocol':'open_router_decisions','endpoint':'https://openrouter.ai/api/alpha/decisions',
            'model':'perplexity/pplx-decider-v1.2','expected_model':'perplexity/pplx-decider-v1.2-20261016',
            'upstream':'perplexity','max_input_bytes':65536,'timeout_secs':90},
        'expected_actual':{'model':'perplexity/pplx-decider-v1.2-20261016','provider':'Perplexity','revision':'',
            'artifact_sha256':'','identity_evidence':'provider_reported_version'},
        'fixtures':fixtures,'expected_answers':expected,'gold_items':gold,
        'resource_fixture_id':panel['id'],'stress_fixture_id':stress['id'],
        'seen_regression_fixture_id':seen['id'],'seen_regression_provenance':regression['provenance']}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',required=True)
    args=parser.parse_args();path=Path(args.output)
    if path.exists():raise ValueError('preserve_existing_authored_spec')
    path.parent.mkdir(parents=True,exist_ok=True)
    value=experiment();path.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({'schema_version':3,'fixtures':len(value['fixtures']),'actual_export_coverage':'required_before_inference'}))


if __name__=='__main__':main()
