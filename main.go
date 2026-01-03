package main

import (
	"context"
	"flag"
	"io/fs"
	"log"
	"net/http"
	"os"

	"github.com/BurntSushi/toml"
	"github.com/bougou/go-ipmi"
	"github.com/gin-contrib/cors"
	"github.com/gin-gonic/gin"
)

var corsAllowAllFlag = flag.Bool("cors-allow-all", false, "Allow all origins")

type Config struct {
	IPMI struct {
		Username     string
		Password     string
		PasswordFile string
	}
	Host map[string]struct {
		Address string
	}
}

type HostState struct {
	Hostname           string
	PowerIsOn          bool
	PowerRestorePolicy string
	Error              string
}

type HostCommand struct {
	Power              *bool
	PowerRestorePolicy *string
}

func ErrorHandler() gin.HandlerFunc {
	return func(c *gin.Context) {
		c.Next()
		if len(c.Errors) > 0 {
			err := c.Errors.Last().Err
			c.JSON(http.StatusInternalServerError, map[string]any{
				"error": err.Error(),
			})
		}
	}
}

func getHostState(ctx context.Context, cs *ClientSet, hostname string) (HostState, error) {
	client, err := cs.Get(hostname)
	if err != nil {
		return HostState{
			Hostname: hostname,
			Error:    err.Error(),
		}, err
	}
	defer cs.Put(client)

	chassis, err := client.GetChassisStatus(ctx)
	if err != nil {
		return HostState{
			Hostname: hostname,
			Error:    err.Error(),
		}, err
	}

	return HostState{
		Hostname:           hostname,
		PowerIsOn:          chassis.PowerIsOn,
		PowerRestorePolicy: chassis.PowerRestorePolicy.String(),
	}, nil
}

func getHosts(cfg *Config, cs *ClientSet) gin.HandlerFunc {
	return func(c *gin.Context) {
		ctx := c.Request.Context()

		result := make(map[string]HostState)
		for hostname := range cfg.Host {
			state, _ := getHostState(ctx, cs, hostname)
			result[hostname] = state
		}

		c.JSON(http.StatusOK, gin.H{
			"hosts": result,
		})
	}
}

func getHost(cs *ClientSet) gin.HandlerFunc {
	return func(c *gin.Context) {
		ctx := c.Request.Context()
		state, err := getHostState(ctx, cs, c.Param("hostname"))
		if err != nil {
			c.Error(err)
			return
		} else {
			c.JSON(http.StatusOK, state)
		}
	}
}

func putHost(cs *ClientSet) gin.HandlerFunc {
	return func(c *gin.Context) {
		var command HostCommand
		if c.BindJSON(&command) != nil {
			return
		}

		hostname := c.Param("hostname")

		ctx := c.Request.Context()
		client, err := cs.Get(hostname)
		if err != nil {
			c.Error(err)
			return
		}
		defer cs.Put(client)

		if command.PowerRestorePolicy != nil {
			var policy ipmi.PowerRestorePolicy
			switch *command.PowerRestorePolicy {
			case "always-on":
				policy = ipmi.PowerRestorePolicyAlwaysOn
			case "previous":
				policy = ipmi.PowerRestorePolicyPrevious
			case "always-off":
				policy = ipmi.PowerRestorePolicyAlwaysOff
			}

			_, err := client.SetPowerRestorePolicy(ctx, policy)
			if err != nil {
				c.Error(err)
				return
			}
		}

		if command.Power != nil {
			var cmd ipmi.ChassisControl
			if *command.Power {
				cmd = ipmi.ChassisControlPowerUp
			} else {
				cmd = ipmi.ChassisControlPowerDown
			}

			_, err := client.ChassisControl(ctx, cmd)
			if err != nil {
				c.Error(err)
				return
			}
		}
	}
}

func readConfig(path string) *Config {
	var config Config
	_, err := toml.DecodeFile(path, &config)
	if err != nil {
		log.Fatal(err)
	}

	if config.IPMI.PasswordFile != "" {
		password, err := os.ReadFile(config.IPMI.PasswordFile)
		if err != nil {
			log.Fatal(err)
		}
		config.IPMI.Password = string(password)
	}

	return &config
}

func main() {
	flag.Parse()

	config := readConfig(flag.Arg(0))
	cs := NewClientSet(config)

	assets, err := fs.Sub(static(), "assets")
	if err != nil {
		log.Fatal(err)
	}

	router := gin.Default()
	router.Use(ErrorHandler())
	if *corsAllowAllFlag {
		router.Use(cors.Default())
	}

	// https://github.com/gin-gonic/gin/issues/2654
	router.GET("/", func(c *gin.Context) {
		c.FileFromFS("/", http.FS(static()))
	})

	router.StaticFS("/assets", http.FS(assets))
	router.GET("/hosts", getHosts(config, cs))
	router.GET("/host/:hostname", getHost(cs))
	router.PUT("/host/:hostname", putHost(cs))

	router.Run()
}
