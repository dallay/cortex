"""Convierte el borrador C1 a hipótesis comparables sin ejecutar modelos."""
import argparse
import json
from pathlib import Path
import re
import sys

import c1_corpus_draft as draft
from compare_minilm_c1 import validate_labels


def convert(corpus, draft_labels):
    """Valida las fuentes y deriva referencias; no declara aceptación C1."""
    draft.validate(corpus=corpus, labels=draft_labels)
    batches = {batch['id']: batch['texts'] for batch in corpus['batches']}

    def refs(*names):
        if any(name not in batches for name in names):
            raise ValueError('faltan batches para coverage')
        return [f"{name}/{index}"
                for name in names for index in range(len(batches[name]))]

    lexical = batches['lexical_es']
    words = [set(re.findall(r'\w+', text.casefold(), flags=re.UNICODE))
             for text in lexical[:2]]
    if words[0] & words[1]:
        raise ValueError('lexical_es 0/1 contiene palabras comunes')

    coverage = {
        'spanish': refs('semantic_es', 'lexical_es'),
        'english': refs('semantic_en'),
        'paraphrase_candidates': [
            f"{item['batch']}/{item[role]}"
            for item in draft_labels['triplets'] for role in ('query', 'positive')],
        'lexical_negatives': [
            f"{item['batch']}/{item['negative']}"
            for item in draft_labels['triplets']],
        'paraphrases_no_shared_words': refs('lexical_es')[:2],
        'unicode': refs('unicode'),
        'empty': [f"{name}/{index}"
                  for name, texts in batches.items() for index, text in enumerate(texts)
                  if text == ''],
        'rust': refs('rust'),
        'cypher': refs('cypher'),
        'technical_decisions': refs('decision'),
        'boundary_candidates': refs(draft_labels['boundary_batch']),
        'prefix_tail': refs('prefix_tail'),
        'batch_reordering': refs('mixed', 'reordered'),
    }
    by_text = {}
    for name, texts in batches.items():
        for index, text in enumerate(texts):
            by_text.setdefault(text, []).append(f"{name}/{index}")
    result = {
        'schema_version': 1,
        'corpus_id': corpus['corpus_id'],
        'reference_format': 'batch_id/zero_based_index',
        'status': 'agent_hypotheses_not_results',
        'label_origin_es': (
            'Hipótesis generadas por agente, no humanas ni resultados medidos. '
            'La ausencia literal de palabras comunes se comprueba con regex Unicode '
            'y casefold en lexical_es 0/1; no demuestra suficiencia semántica.'),
        'triplets': [
            dict(id=f'hypothesis_{index + 1}',
                 rationale_es='Relación propuesta como hipótesis del agente; no resultados ni validación humana.',
                 **{role: f"{item['batch']}/{item[role]}"
                    for role in ('query', 'positive', 'negative')})
            for index, item in enumerate(draft_labels['triplets'])],
        'coverage': coverage,
        'equal_text_groups': [group for group in by_text.values() if len(group) > 1],
        'length_hypotheses': [],
        'length_note_es': (
            'Sin afirmaciones de longitud: el campo legacy hello_repetitions no '
            'describe planet. Las fronteras verificadas deben documentarse en un '
            'receipt de tokenizer separado cuando exista; esta conversión no '
            'tokeniza ni prueba truncamiento.'),
    }
    validate_labels(result, corpus)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--corpus', type=Path, required=True)
    parser.add_argument('--draft-labels', type=Path, required=True)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        corpus = json.loads(args.corpus.read_text(encoding='utf-8'))
        source = json.loads(args.draft_labels.read_text(encoding='utf-8'))
        rendered = json.dumps(convert(corpus, source), ensure_ascii=False, indent=2) + '\n'
        if args.output is None:
            sys.stdout.write(rendered)
        else:
            with args.output.open('x', encoding='utf-8') as stream:
                stream.write(rendered)
    except (OSError, ValueError) as error:
        parser.exit(1, f'Error: {error}\n')


if __name__ == '__main__':
    main()
