# Embedding Pipeline

How raw document and source-code sections are turned into dense vectors and
inserted into the nearest-neighbour index.

## Batch Embedding and Normalization

The ingestion pipeline embeds sections in batches rather than one at a time,
because the dominant cost of running a transformer embedding model on a GPU is
the fixed per-call overhead of moving data onto the device and launching the
kernel, not the marginal cost of an additional row in the batch. To keep the
GPU saturated the pipeline collects section texts until it has filled a batch,
sorts that batch by token length so that padding is minimized within each
group, and only then issues a single forward pass. Length-sorted batching
matters because every sequence in a batch is padded up to the length of the
longest member; mixing a 12-token snippet with a 400-token function in the same
batch wastes most of the compute on padding tokens that contribute nothing to
the result. After the forward pass the model returns one vector per input. Each
vector is L2-normalized — divided by its own Euclidean norm — so that all
stored vectors live on the unit hypersphere. Normalization is what lets the
search layer use a plain dot product as a stand-in for cosine similarity, which
is both faster to compute and friendlier to the approximate index, since the
index can assume every vector has unit length. A vector whose norm is zero (an
all-zero or non-finite embedding) is never inserted: it would otherwise collapse
the cosine geometry and return as a false match to every query, so the pipeline
detects and skips it, recording a warning rather than poisoning the index.

## Sequence Length and Truncation

Every embedding model has a maximum input sequence length, measured in tokens
of the model's own tokenizer, beyond which the input is silently truncated. For
the legacy default model that cap is 256 word-piece tokens, even though the
published tokenizer configuration historically shipped with a tighter 128-token
limit that dropped content without warning. Sections longer than the cap lose
their tail entirely, so a long function whose signature appears at the top but
whose key logic appears halfway down can become unretrievable for queries that
describe that logic. The durable fix is to chunk long sections along structural
boundaries before embedding rather than letting the tokenizer truncate them.
