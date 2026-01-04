import * as React from 'react';
import { Tooltip, Table, TableHead, TableBody, TableRow, TableCell, IconButton, Collapse } from '@mui/material';
import PowerSettingsNewIcon from '@mui/icons-material/PowerSettingsNew'
import KeyboardArrowDownIcon from '@mui/icons-material/KeyboardArrowDown';
import KeyboardArrowUpIcon from '@mui/icons-material/KeyboardArrowUp';
import useSWR from 'swr'
import './App.css'

const fetcher = (url: string) => fetch(url).then(res => res.json());

async function setPowerState(hostname: string, state: boolean) {
  await fetch(`${import.meta.env.VITE_API_URL || ""}/host/${hostname}`, {
    method: "PUT",
    body: JSON.stringify({ power: state }),
    headers: {
      'Content-Type': 'application/json'
    },
  });
}

function SensorRow({ name, value }: {name: string, value: string}) {
  return (
    <TableRow>
      <TableCell>{name}</TableCell>
      <TableCell>{value}</TableCell>
    </TableRow>);
}

function HostRow({ hostname, data }: {hostname: string, data: any}) {
  const [open, setOpen] = React.useState(false);

  console.log(data.sensors);
  console.log(Object.entries(data.sensors)
                    .toSorted(([k1, _v1], [k2, _v2]) => k1.localeCompare(k2)));
  return <>
    <TableRow>
      <TableCell>
        <IconButton aria-label="expand row" size="small" onClick={() => setOpen(!open)} >
          {open ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
        </IconButton>
      </TableCell>
      <TableCell>{hostname}</TableCell>
      <TableCell>{data.error ? "Unavailable" : (data.power_is_on ? "On" : "Off")}</TableCell>
      <TableCell>
        <Tooltip title={data.power_is_on ? "Power Off" : "Power On"}>
          <IconButton
            disabled={!!data.error}
            color={data.power_is_on ? "error" : "success"}
            onClick={async () => { await setPowerState(hostname, !data.power_is_on); } }
          >
            <PowerSettingsNewIcon/>
          </IconButton>
        </Tooltip>
      </TableCell>
    </TableRow>
    <TableRow>
      <TableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={4}>
        <Collapse in={open} timeout="auto">
          <Table>
            <TableBody>
              { Object.entries(data.sensors)
                    .toSorted(([k1, _v1], [k2, _v2]) => k1.localeCompare(k2))
                    .map(([k,v]) => <SensorRow name={k} value={v as string} key={k} />) }
            </TableBody>
          </Table>
        </Collapse>
      </TableCell>
    </TableRow>
  </>;
}

function App() {
  const { data } = useSWR(`${import.meta.env.VITE_API_URL || ""}/hosts`, fetcher, { refreshInterval: 5000 })

  return (
    <>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell></TableCell>
            <TableCell>Hostname</TableCell>
            <TableCell>Status</TableCell>
            <TableCell>Actions</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          { data &&
              Object.entries(data.hosts)
                    .toSorted(([k1, _v1], [k2, _v2]) => k1.localeCompare(k2))
                    .map(([k,v]) => <HostRow hostname={k} data={v} key={k}/>) }
        </TableBody>
      </Table>
    </>
  )

}

export default App
