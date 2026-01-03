import { Tooltip, Table, TableHead, TableBody, TableRow, TableCell, IconButton } from '@mui/material';
import PowerSettingsNewIcon from '@mui/icons-material/PowerSettingsNew'
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

function hostRow(hostname: string, data: any) {
  return (<TableRow key={hostname}>
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
  </TableRow>);
}

function App() {
  const { data } = useSWR(`${import.meta.env.VITE_API_URL || ""}/hosts`, fetcher, { refreshInterval: 5000 })

  return (
    <>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell>Hostname</TableCell>
            <TableCell>Status</TableCell>
            <TableCell>Actions</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          { data &&
              Object.entries(data.hosts)
                    .toSorted(([k1, _v1], [k2, _v2]) => k1.localeCompare(k2))
                    .map(([k,v]) => hostRow(k,v)) }
        </TableBody>
      </Table>
    </>
  )

}

export default App
