// Package pdftest writes small PDFs for tests and inspects their pages.
package pdftest

import (
	"bytes"
	"fmt"
	"image"
	"image/jpeg"
	"math"
	"math/rand/v2"
	"os"
	"strings"
	"testing"

	"github.com/pdfcpu/pdfcpu/pkg/api"
)

// Write writes a minimal PDF with one page per width, so tests can identify pages by
// their width.
func Write(t *testing.T, path string, widths ...int) {
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

// WriteImage writes a one-page PDF that shows a photo-like size x size JPEG at 72 DPI,
// so Ghostscript never downsamples it and only JPEG re-encoding changes its size.
func WriteImage(t *testing.T, path string, size int) {
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

// PageWidths returns the width of each page of the PDF at path.
func PageWidths(t *testing.T, path string) []int {
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
