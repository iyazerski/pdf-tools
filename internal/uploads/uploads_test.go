package uploads

import (
	"os"
	"path/filepath"
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
	return s.Put(dir, filepath.Join(dir, "in.pdf"), 1), dir
}

func assertDirExists(t *testing.T, dir string, want bool) {
	t.Helper()
	_, err := os.Stat(dir)
	if exists := err == nil; exists != want {
		t.Errorf("directory %s exists = %v; want %v (stat error: %v)", dir, exists, want, err)
	}
}

func TestSweepDeletesExpiredUploads(t *testing.T) {
	s := NewStore(time.Minute, 10)
	id, dir := putOne(t, s)

	s.sweep(time.Now().Add(61 * time.Second))

	if _, ok := s.Get(id); ok {
		t.Error("expired upload is still in the store")
	}
	assertDirExists(t, dir, false)
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
