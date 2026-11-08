# Retrieval judgment pilot rubric

## Scope and version

This document defines rubric version `muninn-retrieval-v1.0` for the 12-pair
retrieval judgment pilot. Each of six query anchors has two candidate documents.
The corpus is a separate draft. It does not change or freeze C1 source material
or labels.

## Grades

Assign one integer grade to each query-candidate pair:

| Grade | Operational definition |
| --- | --- |
| 0 | Irrelevant or contradictory. The document does not address the query, or it recommends an outcome that conflicts with the query's stated need. |
| 1 | Topical but not useful. The document concerns the same subject but does not provide actionable information for the query. |
| 2 | Partially useful. The document provides applicable information, but it is incomplete, indirect, or needs substantial additional context. |
| 3 | Directly and sufficiently useful. The document addresses the query and provides enough information to satisfy it without substantial additional context. |

Grades 2 and 3 count as relevant for Recall and MRR. The existing metric
implementation continues to use `grade >= 2` for relevance.

## Blind procedure

1. Use only the neutral query and candidate IDs and their text in the separate
   pilot corpus. Do not consult the C1 source labels, triplet metadata, previous
   judgments, or candidate provenance while assigning grades.
2. Judge each candidate independently against the query. Do not compare a
   candidate to the other candidate when assigning its grade.
3. Fill only the `reviewer_id` and `grade` columns in the CSV template. Use the
   same pseudonymous reviewer ID in every row. Enter one integer from 0 through
   3 for each of the 12 rows. Do not add free-form rationale or change any
   other column.
4. Use a pseudonymous reviewer ID in the form `reviewer-` followed by 4–32
   lowercase letters or digits. Do not enter a name, email address, account
   identifier, or other personal data.
5. Record rubric version `muninn-retrieval-v1.0` and adjudication metadata
   `single_reviewer_not_adjudicated` with reviewer count 1. This pilot has one
   reviewer; it has no independent second review or adjudication.
6. Convert the completed CSV to the strict JSON annotation schema with
   `annotations_from_csv(csv_text, corpus)` from `dev/muninn/retrieval_quality.py`.
   Pass the complete CSV text as `csv_text` and the loaded pilot corpus object as
   `corpus`. The function validates the template header, all 12 query-candidate
   pairs, metadata, corpus hash, and grades before it returns the annotation
   object. This module has no CSV-import CLI.
7. At the later import or run step, calculate and report the raw CSV file's
   SHA-256 separately from the returned annotation object. Do not inject the raw
   CSV hash into the annotation object or put a self-hash in the CSV.

The deterministic candidate shuffle assigns neutral candidate IDs and does not
encode prior roles. The corpus contains no source batch names, role fields,
polarity labels, rationale, or agent-judgment fields.

## Interpretation limits

This small pilot is exploratory. It does not evidence high parity, establish
production retrieval quality, or support acceptance claims. Its judgments must
not be represented as independent validation of the source hypotheses.
