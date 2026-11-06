// Package uploads keeps uploaded PDFs on disk for a limited time.
package uploads

import (
	"context"
	"log/slog"
	"os"
	"sync"
	"time"
	"uuid"
)

const sweepInterval = time.Minute

// Upload is an uploaded PDF in its own directory.
//
// Get hands out an Upload with a reference, and its directory stays on disk until every
// reference is released, even if the upload expires or is removed meanwhile.
type Upload struct {
	Dir     string
	PDFPath string
	Pages   int

	// refs and removed are guarded by the owning Store's mutex.
	refs    int
	removed bool

	// thumbsLock is a context-aware mutex that guards thumbsDone.
	thumbsLock chan struct{}
	thumbsDone bool
}

// EnsureThumbs calls render unless an earlier call succeeded.
//
// Concurrent callers wait for the running render instead of starting their own. A failed
// render is retried by the next caller.
func (u *Upload) EnsureThumbs(ctx context.Context, render func(context.Context) error) error {
	select {
	case u.thumbsLock <- struct{}{}:
	case <-ctx.Done():
		return ctx.Err()
	}
	defer func() { <-u.thumbsLock }()

	if u.thumbsDone {
		return nil
	}
	if err := render(ctx); err != nil {
		return err
	}
	u.thumbsDone = true
	return nil
}

type entry struct {
	upload     *Upload
	lastAccess time.Time
}

// Store maps upload IDs to uploads, deleting ones that are idle for longer than its TTL.
type Store struct {
	ttl        time.Duration
	maxEntries int

	mu      sync.Mutex
	entries map[string]*entry
}

// NewStore returns a Store that expires uploads idle for ttl and keeps at most maxEntries.
func NewStore(ttl time.Duration, maxEntries int) *Store {
	return &Store{ttl: ttl, maxEntries: maxEntries, entries: make(map[string]*entry)}
}

// Put stores the PDF at pdfPath and returns its new upload ID.
//
// The store takes ownership of dir and deletes it when the upload is gone. When the store
// is full, the least recently used uploads are evicted: bounded disk use matters more than
// keeping old uploads, which the client can add again.
func (s *Store) Put(dir, pdfPath string, pages int) string {
	id := uuid.New().String()
	u := &Upload{Dir: dir, PDFPath: pdfPath, Pages: pages, thumbsLock: make(chan struct{}, 1)}

	s.mu.Lock()
	var evicted []string
	for len(s.entries) >= s.maxEntries {
		oldest := ""
		for key, e := range s.entries {
			if oldest == "" || e.lastAccess.Before(s.entries[oldest].lastAccess) {
				oldest = key
			}
		}
		evicted = append(evicted, s.removeLocked(oldest)...)
	}
	s.entries[id] = &entry{upload: u, lastAccess: time.Now()}
	s.mu.Unlock()

	removeDirs(evicted)
	return id
}

// Get returns the upload with id and marks it as used. The caller must pass it to Release
// when done.
func (s *Store) Get(id string) (*Upload, bool) {
	now := time.Now()
	s.mu.Lock()
	e, ok := s.entries[id]
	if !ok {
		s.mu.Unlock()
		return nil, false
	}
	if s.expired(e, now) {
		dirs := s.removeLocked(id)
		s.mu.Unlock()
		removeDirs(dirs)
		return nil, false
	}
	e.lastAccess = now
	e.upload.refs++
	s.mu.Unlock()
	return e.upload, true
}

// Release gives back an upload returned by Get.
func (s *Store) Release(u *Upload) {
	s.mu.Lock()
	u.refs--
	deleteDir := u.removed && u.refs == 0
	s.mu.Unlock()

	if deleteDir {
		removeDirs([]string{u.Dir})
	}
}

// Remove deletes the upload with id, if it exists.
func (s *Store) Remove(id string) {
	s.mu.Lock()
	dirs := s.removeLocked(id)
	s.mu.Unlock()
	removeDirs(dirs)
}

// Run deletes expired uploads periodically until ctx is done, so files don't outlive the
// TTL while the app is idle. Then it deletes all remaining uploads.
func (s *Store) Run(ctx context.Context) {
	ticker := time.NewTicker(sweepInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			s.removeAll()
			return
		case now := <-ticker.C:
			s.sweep(now)
		}
	}
}

func (s *Store) sweep(now time.Time) {
	s.mu.Lock()
	var dirs []string
	for id, e := range s.entries {
		if s.expired(e, now) {
			dirs = append(dirs, s.removeLocked(id)...)
		}
	}
	s.mu.Unlock()
	removeDirs(dirs)
}

func (s *Store) removeAll() {
	s.mu.Lock()
	var dirs []string
	for id := range s.entries {
		dirs = append(dirs, s.removeLocked(id)...)
	}
	s.mu.Unlock()
	removeDirs(dirs)
}

// removeLocked drops the entry with id and returns its directory when it's no longer in
// use. The caller must hold s.mu and delete the returned directories after unlocking.
func (s *Store) removeLocked(id string) []string {
	e, ok := s.entries[id]
	if !ok {
		return nil
	}
	delete(s.entries, id)
	e.upload.removed = true
	if e.upload.refs > 0 {
		return nil
	}
	return []string{e.upload.Dir}
}

func (s *Store) expired(e *entry, now time.Time) bool {
	return now.Sub(e.lastAccess) > s.ttl
}

func removeDirs(dirs []string) {
	for _, dir := range dirs {
		if err := os.RemoveAll(dir); err != nil {
			slog.Error("delete upload directory", "dir", dir, "error", err)
		}
	}
}
