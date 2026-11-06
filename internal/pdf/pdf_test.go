package pdf

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"testing"
	"time"

	"github.com/iyazerski/pdf-tools/internal/pdftest"
)

func TestPageCount(t *testing.T) {
	dir := t.TempDir()
	p := NewProcessor(time.Minute, 2)

	valid := filepath.Join(dir, "valid.pdf")
	pdftest.Write(t, valid, 101, 102, 103)
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
	pdftest.Write(t, a, 101, 102, 103, 104)
	pdftest.Write(t, b, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211)
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
			name:       "whole and partial documents are merged without joining",
			layout:     slices.Concat(refs("b", 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11), refs("a", 3, 1)),
			quality:    LosslessQuality,
			wantWidths: []int{201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 103, 101},
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
			if got := pdftest.PageWidths(t, out); !slices.Equal(got, tt.wantWidths) {
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
	pdftest.WriteImage(t, in, 400)
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
