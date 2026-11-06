package pdf

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"image"
	"image/jpeg"
	"math"
	"math/rand/v2"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/pdfcpu/pdfcpu/pkg/api"
)

// writePDF writes a minimal PDF with one page per width, so tests can identify pages by
// their width.
func writePDF(t *testing.T, path string, widths ...int) {
	t.Helper()
	kids := make([]string, len(widths))
	for i := range widths {
		kids[i] = fmt.Sprintf("%d 0 R", i+3)
	}
	objects := []string{
		"<< /Type /Catalog /Pages 2 0 R >>",
		fmt.Sprintf("<< /Type /Pages /Kids [%s] /Count %d >>", strings.Join(kids, " "), len(widths)),
	}
	for _, w := range widths {
		objects = append(objects, fmt.Sprintf("<< /Type /Page /Parent 2 0 R /Resources << >> /MediaBox [0 0 %d 100] >>", w))
	}
	writeObjects(t, path, objects)
}

// writeImagePDF writes a one-page PDF that shows a photo-like size x size JPEG at 72 DPI,
// so Ghostscript never downsamples it and only JPEG re-encoding changes its size.
func writeImagePDF(t *testing.T, path string, size int) {
	t.Helper()
	// Smooth waves with mild noise: Ghostscript keeps pure noise lossless, because it
	// only picks JPEG for images that look like photos.
	img := image.NewRGBA(image.Rect(0, 0, size, size))
	rng := rand.New(rand.NewPCG(1, 2))
	for y := range size {
		for x := range size {
			wave := 100 * math.Sin(float64(x)/20) * math.Cos(float64(y)/25)
			for c := range 3 {
				img.Pix[img.PixOffset(x, y)+c] = uint8(128 + wave + float64(rng.IntN(41)-20) - float64(c*10))
			}
			img.Pix[img.PixOffset(x, y)+3] = 255
		}
	}
	var jpg bytes.Buffer
	if err := jpeg.Encode(&jpg, img, &jpeg.Options{Quality: 95}); err != nil {
		t.Fatal(err)
	}
	content := fmt.Sprintf("q %d 0 0 %d 0 0 cm /Im0 Do Q", size, size)
	writeObjects(t, path, []string{
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		fmt.Sprintf("<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R /MediaBox [0 0 %d %d] >>", size, size),
		fmt.Sprintf("<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length %d >>\nstream\n%s\nendstream", size, size, jpg.Len(), jpg.Bytes()),
		fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(content), content),
	})
}

// writeObjects writes a PDF whose objects are numbered from 1 in order, with object 1 as
// its catalog.
func writeObjects(t *testing.T, path string, objects []string) {
	t.Helper()
	var b bytes.Buffer
	var offsets []int
	b.WriteString("%PDF-1.4\n")
	for _, body := range objects {
		offsets = append(offsets, b.Len())
		fmt.Fprintf(&b, "%d 0 obj\n%s\nendobj\n", len(offsets), body)
	}
	xref := b.Len()
	fmt.Fprintf(&b, "xref\n0 %d\n0000000000 65535 f \n", len(offsets)+1)
	for _, off := range offsets {
		fmt.Fprintf(&b, "%010d 00000 n \n", off)
	}
	fmt.Fprintf(&b, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(offsets)+1, xref)

	if err := os.WriteFile(path, b.Bytes(), 0o600); err != nil {
		t.Fatal(err)
	}
}

func pageWidths(t *testing.T, path string) []int {
	t.Helper()
	dims, err := api.PageDimsFile(t.Context(), path)
	if err != nil {
		t.Fatalf("read page dims of %s: %v", path, err)
	}
	widths := make([]int, len(dims))
	for i, d := range dims {
		widths[i] = int(d.Width)
	}
	return widths
}

func TestPageCount(t *testing.T) {
	dir := t.TempDir()
	p := NewProcessor(time.Minute, 2)

	valid := filepath.Join(dir, "valid.pdf")
	writePDF(t, valid, 101, 102, 103)
	pages, err := p.PageCount(t.Context(), valid)
	if err != nil || pages != 3 {
		t.Errorf("PageCount(valid) = %d, %v; want 3, nil", pages, err)
	}

	invalid := filepath.Join(dir, "invalid.pdf")
	if err := os.WriteFile(invalid, []byte("%PDF-1.4\nnot really a pdf"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := p.PageCount(t.Context(), invalid); !errors.Is(err, ErrInvalidPDF) {
		t.Errorf("PageCount(invalid) error = %v; want ErrInvalidPDF", err)
	}
}

func TestMerge(t *testing.T) {
	dir := t.TempDir()
	a := filepath.Join(dir, "a.pdf")
	b := filepath.Join(dir, "b.pdf")
	writePDF(t, a, 101, 102, 103, 104)
	writePDF(t, b, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211)
	inputs := map[string]Input{"a": {Path: a, Pages: 4}, "b": {Path: b, Pages: 11}}

	refs := func(upload string, pages ...int) []PageRef {
		out := make([]PageRef, len(pages))
		for i, page := range pages {
			out[i] = PageRef{Upload: upload, Page: page}
		}
		return out
	}

	tests := []struct {
		name       string
		layout     []PageRef
		quality    int
		wantWidths []int
		// wantInput is set when the result must be that input file, unchanged.
		wantInput string
	}{
		{
			name:      "one whole document is returned as-is",
			layout:    refs("a", 1, 2, 3, 4),
			quality:   LosslessQuality,
			wantInput: a,
		},
		{
			name:       "pages of one document keep the given order",
			layout:     refs("a", 3, 1, 1),
			quality:    LosslessQuality,
			wantWidths: []int{103, 101, 101},
		},
		{
			name:       "documents interleave in layout order",
			layout:     slices.Concat(refs("a", 1, 2, 3), refs("b", 10, 11), refs("a", 4)),
			quality:    LosslessQuality,
			wantWidths: []int{101, 102, 103, 210, 211, 104},
		},
		{
			name:       "lossy merge interleaves documents in layout order",
			layout:     slices.Concat(refs("a", 1, 2, 3), refs("b", 10, 11), refs("a", 4)),
			quality:    50,
			wantWidths: []int{101, 102, 103, 210, 211, 104},
		},
		{
			name:       "whole documents are merged",
			layout:     slices.Concat(refs("b", 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11), refs("a", 1, 2, 3, 4)),
			quality:    LosslessQuality,
			wantWidths: []int{201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 101, 102, 103, 104},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := exec.LookPath("gs"); err != nil && tt.quality < LosslessQuality {
				t.Skip("lossy merges need Ghostscript (gs)")
			}
			out, err := NewProcessor(time.Minute, 2).Merge(t.Context(), t.TempDir(), inputs, tt.layout, tt.quality)
			if err != nil {
				t.Fatalf("Merge: %v", err)
			}
			if tt.wantInput != "" {
				if out != tt.wantInput {
					t.Errorf("Merge = %s; want input %s", out, tt.wantInput)
				}
				return
			}
			if got := pageWidths(t, out); !slices.Equal(got, tt.wantWidths) {
				t.Errorf("page widths = %v; want %v", got, tt.wantWidths)
			}
		})
	}
}

func TestLowerQualityShrinksImages(t *testing.T) {
	if _, err := exec.LookPath("gs"); err != nil {
		t.Skip("lossy merges need Ghostscript (gs)")
	}
	in := filepath.Join(t.TempDir(), "image.pdf")
	writeImagePDF(t, in, 400)
	inputs := map[string]Input{"a": {Path: in, Pages: 1}}
	layout := []PageRef{{Upload: "a", Page: 1}}

	size := func(quality int) int64 {
		t.Helper()
		out, err := NewProcessor(time.Minute, 2).Merge(t.Context(), t.TempDir(), inputs, layout, quality)
		if err != nil {
			t.Fatalf("Merge(quality %d): %v", quality, err)
		}
		info, err := os.Stat(out)
		if err != nil {
			t.Fatal(err)
		}
		return info.Size()
	}
	if low, high := size(MinQuality), size(90); low*2 > high {
		t.Errorf("output size at quality %d = %d, at 90 = %d; want under half", MinQuality, low, high)
	}
}

func TestJobWaitingTooLongIsBusy(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := NewProcessor(time.Minute, 1)
		running, release := make(chan struct{}), make(chan struct{})
		go func() {
			_ = p.do(t.Context(), "first", func(context.Context) error {
				close(running)
				<-release
				return nil
			})
		}()
		<-running

		err := p.do(t.Context(), "second", func(context.Context) error { return nil })
		if !errors.Is(err, ErrBusy) {
			t.Errorf("do while the only slot is taken = %v; want ErrBusy", err)
		}
		close(release)
	})
}

func TestPrefixWriter(t *testing.T) {
	w := &prefixWriter{max: 4}
	for _, s := range []string{"ab", "cé", "tail"} {
		if n, err := w.Write([]byte(s)); n != len(s) || err != nil {
			t.Fatalf("Write(%q) = %d, %v; want %d, nil", s, n, err, len(s))
		}
	}
	// "é" is 2 bytes, and only its first byte fits.
	if got, want := w.String(), "abc…"; got != want {
		t.Errorf("String() = %q; want %q", got, want)
	}
}

func TestPageSelection(t *testing.T) {
	tests := []struct {
		pages []int
		want  []string
	}{
		{pages: []int{1, 2, 3, 10, 11, 4}, want: []string{"1-3", "10-11", "4"}},
		{pages: []int{1, 3}, want: []string{"1", "3"}},
		{pages: []int{3, 2, 1}, want: []string{"3", "2", "1"}},
	}
	for _, tt := range tests {
		if got := pageSelection(tt.pages); !slices.Equal(got, tt.want) {
			t.Errorf("pageSelection(%v) = %v; want %v", tt.pages, got, tt.want)
		}
	}
}
