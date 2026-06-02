// Package resilience provides helpers for tolerating transient failures when
// calling unreliable downstream services. It is part of the code-heavy
// evaluation corpus (eval/corpus-code) used to benchmark embedders on
// text-to-code retrieval.
package resilience

import (
	"context"
	"math"
	"time"
)

// Operation is a unit of work that may fail transiently and is safe to repeat.
type Operation func(ctx context.Context) error

// RetryWithBackoff repeatedly invokes op until it succeeds, the context is
// cancelled, or the attempt budget is exhausted.
//
// Between failed attempts it waits for an exponentially growing delay
// (base * 2^attempt) so that a struggling dependency is not hammered. This is
// the classic "exponential backoff" strategy for retrying flaky network calls.
func RetryWithBackoff(ctx context.Context, attempts int, base time.Duration, op Operation) error {
	var err error
	for attempt := 0; attempt < attempts; attempt++ {
		if err = op(ctx); err == nil {
			return nil
		}
		delay := time.Duration(float64(base) * math.Pow(2, float64(attempt)))
		select {
		case <-time.After(delay):
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return err
}

// CircuitBreaker trips open after consecutive failures cross a threshold,
// short-circuiting further calls to give a failing dependency time to recover.
type CircuitBreaker struct {
	threshold int
	failures  int
	open      bool
}

// NewCircuitBreaker returns a closed breaker that opens after `threshold`
// consecutive failures.
func NewCircuitBreaker(threshold int) *CircuitBreaker {
	return &CircuitBreaker{threshold: threshold}
}

// Allow reports whether a call may proceed. Once the breaker is open it stays
// open until Reset is called.
func (c *CircuitBreaker) Allow() bool {
	return !c.open
}

// Record updates the breaker with the outcome of a call, tripping it open when
// too many failures pile up and clearing the count on any success.
func (c *CircuitBreaker) Record(success bool) {
	if success {
		c.failures = 0
		return
	}
	c.failures++
	if c.failures >= c.threshold {
		c.open = true
	}
}

// Reset closes the breaker and clears its failure count.
func (c *CircuitBreaker) Reset() {
	c.failures = 0
	c.open = false
}
