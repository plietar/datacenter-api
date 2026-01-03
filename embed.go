//go:build embed

package main

import (
	"embed"
	"io/fs"
	"log"
)

//go:embed web/dist
var embeddedStatic embed.FS

func static() fs.FS {
	fs, err := fs.Sub(embeddedStatic, "web/dist")
	if err != nil {
		log.Fatal(err)
	}

	return fs
}
