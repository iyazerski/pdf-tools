package main

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestShutdownCancelsAndWaitsForSlowRequests(t *testing.T) {
	requestsCtx, cancelRequests := context.WithCancel(context.Background())
	defer cancelRequests()
	started, finished := make(chan struct{}), make(chan struct{})
	ts := httptest.NewUnstartedServer(http.HandlerFunc(func(_ http.ResponseWriter, r *http.Request) {
		close(started)
		<-r.Context().Done()
		// Stands in for a job that takes a moment to stop.
		time.Sleep(50 * time.Millisecond)
		close(finished)
	}))
	ts.Config.BaseContext = func(net.Listener) context.Context { return requestsCtx }
	ts.Start()
	defer ts.Close()

	go func() {
		req, _ := http.NewRequestWithContext(t.Context(), http.MethodGet, ts.URL, nil)
		if resp, err := ts.Client().Do(req); err == nil {
			resp.Body.Close()
		}
	}()
	<-started

	if err := shutdown(ts.Config, cancelRequests, 50*time.Millisecond, 5*time.Second); err != nil {
		t.Fatalf("shutdown = %v; want nil", err)
	}
	select {
	case <-finished:
	default:
		t.Error("shutdown returned before the canceled request finished")
	}
}
