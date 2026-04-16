# Making AI Coding Agents Faster and Smarter

*A short talk on the ideas behind context engineering for agents. ~5 minutes to read aloud.*

---

## The Problem

AI coding assistants are impressive until they touch a real codebase.

Ask one to fix a bug, and watch what it does. It runs text searches. It reads three files it didn't need. It scrolls through a two-thousand-line module looking for one function. It forgets what it learned five minutes ago and reads the same file again.

Every one of those steps burns tokens. Every token costs money and — more importantly — takes up space in the agent's context window. Context windows are small. Once they fill up with noise, the agent starts making worse decisions: it misses the bug, rewrites something it shouldn't, or confidently ships a change that breaks production.

The bottleneck isn't the model anymore. It's how the model sees your code.

---

## The Reframe

Here's the shift in thinking: an agent's context window is basically a CPU cache.

- It's small and expensive.
- It fills up fast.
- What's in it determines how fast and how accurate the next operation is.
- When it's full, you have to decide what to throw out.

Computer science solved this problem forty years ago. CPUs don't just hope the right data is in cache — they have cache controllers that predict what's next, prefetch it, and evict what's stale.

The same discipline applies to agents. You don't make an agent smarter by giving it a bigger model. You make it smarter by managing what's in front of it.

---

## The Core Ideas

Four ideas, each borrowed from a well-understood part of computer science.

**1. Semantic retrieval instead of text search.**
Text search answers "where does this exact string appear?" That's almost never the real question. The real question is "where do we handle auth token refresh?" — a concept, not a string. Embedding-based retrieval lets the agent ask the question it actually has, and get back the relevant code and docs ranked by meaning. One call, the right answer, no spelunking.

**2. Graph-aware code navigation.**
Source code is a graph — functions call functions, types implement traits, modules import modules. Humans navigate it with IDE features: jump to definition, find all references, show the call hierarchy. For two decades, agents didn't have that. Giving them symbol-level navigation with reference graphs replaces dozens of blind file reads with one targeted lookup.

**3. Predictive prefetching.**
When the agent opens a function, it's almost certainly going to want the callers, the types it uses, and the tests that cover it. A good context layer watches the access pattern and warms those up in advance. This is the same idea as a branch predictor or a disk read-ahead buffer — anticipate, don't react.

**4. Budget-aware eviction and compression.**
When the window starts filling up, something has to go. The naive choice is "drop the oldest." A better choice is "drop the least relevant to the current task" — or better still, summarize it so the gist survives even when the verbatim text doesn't. Treat the context window like a memory hierarchy: hot stuff verbatim, warm stuff compressed, cold stuff evictable.

---

## Why This Makes Agents Better

Three concrete wins.

**The agent reads less and does more.** A task that used to take twenty tool calls takes five. Less waiting, less cost, less chance of the agent getting lost in the weeds.

**The same token budget goes further.** When the context is mostly relevant code instead of mostly scrollback, the model's answers get sharper. You get more capability out of the same model, for the same money.

**It scales to real codebases.** A hundred-thousand-line repo stops being a wall. The agent navigates it the way a senior engineer does — by concept, by symbol, by reference — not by blind text search through a haystack.

---

## How It Fits In

None of this requires training a new model, writing clever prompts, or switching agent frameworks. It's a layer that sits between the agent and the codebase: the agent asks higher-level questions, and the layer translates them into precise retrieval.

The agent you already use, with better tools underneath, is dramatically more effective than the same agent with raw file access. Same model, same prompt, better outcomes — because the information flowing in is cleaner.

---

## The Takeaway

The next leap in agentic development isn't a bigger model. It's better access to the code.

Agents are already smart enough. What they've been missing is the thing every human engineer takes for granted: a way to navigate a codebase without reading all of it.

Treat the context window like a cache. Retrieve by meaning, not by string. Navigate the code graph, not the file tree. Predict what's next. Evict what's stale.

Do that, and the agent you already have becomes the agent you wished it was.

---

*Questions?*
