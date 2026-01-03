package main

import (
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/bougou/go-ipmi"
	"github.com/silenceper/pool"
)

type ClientSet struct {
	cfg *Config

	mu    sync.Mutex
	pools map[string]pool.Pool
}

func clientFactory(cfg *Config, hostname string) func() (interface{}, error) {
	return func() (interface{}, error) {
		data, ok := cfg.Host[hostname]
		if !ok {
			return nil, fmt.Errorf("Unknown hostname: %s", hostname)
		}

		client, err := ipmi.NewClient(data.Address, 623, cfg.IPMI.Username, cfg.IPMI.Password)
		if err != nil {
			return nil, err
		}

		if err := client.Connect(context.Background()); err != nil {
			return nil, err
		}

		return client, nil
	}
}

func NewClientSet(cfg *Config) *ClientSet {
	return &ClientSet{
		cfg:   cfg,
		pools: make(map[string]pool.Pool),
	}
}

type ClientHandle struct {
	*ipmi.Client

	p pool.Pool
}

func (cs *ClientSet) Get(hostname string) (ClientHandle, error) {
	cs.mu.Lock()
	defer cs.mu.Unlock()

	p, ok := cs.pools[hostname]
	if !ok {
		poolConfig := &pool.Config{
			InitialCap: 0,
			MaxIdle:    1,
			MaxCap:     1,
			Factory:    clientFactory(cs.cfg, hostname),
			Close:      func(v interface{}) error { return nil },
			// Close:       close,
			IdleTimeout: 15 * time.Second,
		}

		var err error
		p, err = pool.NewChannelPool(poolConfig)
		if err != nil {
			return ClientHandle{}, err
		}

		cs.pools[hostname] = p
	}

	v, err := p.Get()
	if err != nil {
		return ClientHandle{}, err
	}

	return ClientHandle{v.(*ipmi.Client), p}, nil
}

func (cs *ClientSet) Put(handle ClientHandle) {
	handle.p.Put(handle.Client)
}
