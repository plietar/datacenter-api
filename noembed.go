//go:build !embed

package main

import "io/fs"

type Empty struct{}

func (Empty) Open(name string) (fs.File, error) { return nil, fs.ErrNotExist }

func static() fs.FS {
	return Empty{}
}
