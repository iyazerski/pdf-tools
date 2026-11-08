package uploads

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"testing/synctest"
	"time"
)

func putOne(t *testing.T, s *Store) (id, dir string) {
	t.Helper()
	dir = filepath.Join(t.TempDir(), "upload")
	if err := os.Mkdir(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	id, err := s.Put(dir, filepath.Join(dir, "in.pdf"), 1)
	if err != nil {
		t.Fatal(err)
	}
	return id, dir
}

func assertDirExists(t *testing.T, dir string, want bool) {
	t.Helper()
	_, err := os.Stat(dir)
	if exists := err == nil; exists != want {
		t.Errorf("directory %s exists = %v; want %v (stat error: %v)", dir, exists, want, err)
	}
}

func TestRunDeletesExpiredUploadsThenAllOnStop(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := NewStore(time.Minute, 10)
		expired, expiredDir := putOne(t, s)
		ctx, stop := context.WithCancel(t.Context())
		done := make(chan struct{})
		go func() {
			s.Run(ctx)
			close(done)
		}()

		// The first sweep finds the upload idle for exactly the TTL, so the second deletes it.
		time.Sleep(2*sweepInterval + time.Second)
		synctest.Wait()
		if _, ok := s.Get(expired); ok {
			t.Error("expired upload is still in the store")
		}
		assertDirExists(t, expiredDir, false)

		_, freshDir := putOne(t, s)
		stop()
		<-done
		assertDirExists(t, freshDir, false)

		// Requests that outlive shutdown can't add uploads that nothing would delete.
		if _, err := s.Put(t.TempDir(), "in.pdf", 1); !errors.Is(err, ErrClosed) {
			t.Errorf("Put after stop = %v; want %v", err, ErrClosed)
		}
	})
}

func TestPutEvictsLeastRecentlyUsed(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := NewStore(time.Hour, 2)
		first, _ := putOne(t, s)
		time.Sleep(time.Second)
		second, secondDir := putOne(t, s)
		time.Sleep(time.Second)
		u, _ := s.Get(first)
		s.Release(u)
		time.Sleep(time.Second)
		third, _ := putOne(t, s)

		for id, want := range map[string]bool{first: true, second: false, third: true} {
			u, ok := s.Get(id)
			if ok != want {
				t.Errorf("Get(%s) found = %v; want %v", id, ok, want)
			}
			if ok {
				s.Release(u)
			}
		}
		assertDirExists(t, secondDir, false)
	})
}

func TestRenderRunsOncePerKeyAndRetriesFailures(t *testing.T) {
	s := NewStore(time.Hour, 10)
	id, _ := putOne(t, s)
	u, _ := s.Get(id)
	defer s.Release(u)

	var mu sync.Mutex
	calls := map[string]int{}
	render := func(key string, err error) func(context.Context) error {
		return func(context.Context) error {
			mu.Lock()
			calls[key]++
			mu.Unlock()
			return err
		}
	}

	var wg sync.WaitGroup
	for range 10 {
		wg.Go(func() {
			if err := u.Render(t.Context(), "a", render("a", nil)); err != nil {
				t.Errorf("Render(a) = %v", err)
			}
		})
	}
	wg.Wait()
	failure := errors.New("boom")
	if err := u.Render(t.Context(), "b", render("b", failure)); !errors.Is(err, failure) {
		t.Errorf("failing Render(b) = %v; want %v", err, failure)
	}
	if err := u.Render(t.Context(), "b", render("b", nil)); err != nil {
		t.Errorf("retried Render(b) = %v", err)
	}

	if calls["a"] != 1 || calls["b"] != 2 {
		t.Errorf("render calls = %v; want a: 1, b: 2", calls)
	}
}

func TestRemovedUploadKeepsDirUntilReleased(t *testing.T) {
	s := NewStore(time.Hour, 10)
	id, dir := putOne(t, s)
	u, ok := s.Get(id)
	if !ok {
		t.Fatal("upload not found")
	}

	s.Remove(id)
	assertDirExists(t, dir, true)

	s.Release(u)
	assertDirExists(t, dir, false)
}
