# Research corpus — performance & cross-platform (2026)

Text-extracted PDFs of 2026-vintage scholarly work directly relevant to iris-rs
performance and cross-platform deployment. Indexed by iris.

## ANN / HNSW (vector index)

- `aqr-hnsw.txt` — AQR-HNSW: density-aware quantization + multi-stage rerank on HNSW (arXiv 2602.21600)
- `phnsw-pca.txt` — pHNSW: PCA-based filtering, algorithm-hardware co-design (ASP-DAC 2026)
- `pro-hnsw.txt` — PRO-HNSW: proactive repair for dynamic HNSW indexes (Yonsei DELAB)
- `crouting.txt` — CRouting: skipping redundant distance computations on HNSW/NSG (arXiv 2509.00365)
- `projection-augmented-graph.txt` — 5× QPS-recall over HNSW via projection augmentation (arXiv 2603.06660)
- `crisp-subspace.txt` — CRISP: correlation-resilient indexing via subspace partitioning (arXiv 2603.05180)
- `darth-plus.txt` — DARTH+: declarative recall and quality guarantees for ANN (HAL hal-05566027)
- `ssd-resident-graph.txt` — SSD-resident graph indexing for high-throughput vector search (arXiv 2602.22805)

## Embedding compression

- `smec-matryoshka.txt` — Smec: adaptive dimension selection for Matryoshka representation learning (EMNLP 2025)

## Inference / prefetch (LLM systems)

- `solidattention-fast26.txt` — SolidAttention: low-latency SSD-based serving with speculative prefetching (USENIX FAST '26)
- `lightweight-transformers.txt` — INT8/FP16 quantization, ONNX runtime cross-platform deployment (arXiv 2601.03290)

## Provenance

Downloaded via SerpApi Google Scholar search (`as_ylo=2026`, `scisbd=1`),
converted with `pdftotext -layout`. Original PDFs gitignored via `.iris.toml`.
