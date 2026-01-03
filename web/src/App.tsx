import { Tooltip, Table, TableHead, TableBody, TableRow, TableCell, IconButton } from '@mui/material';
import PowerSettingsNewIcon from '@mui/icons-material/PowerSettingsNew'
import useSWR from 'swr'
import './App.css'

const fetcher = (url: string) => fetch(url).then(res => res.json());

async function setPowerState(hostname: string, state: boolean) {
  await fetch(`${import.meta.env.VITE_API_URL || ""}/host/${hostname}`, {
    method: "PUT",
    body: JSON.stringify({ Power: state }),
  });
}

function hostRow(data: any) {
  return (<TableRow key={data.Hostname}>
    <TableCell>{data.Hostname}</TableCell>
    <TableCell>{data.Error ? "Unavailable" : (data.PowerIsOn ? "On" : "Off")}</TableCell>
    <TableCell>
      <Tooltip title={data.PowerIsOn ? "Power Off" : "Power On"}>
        <IconButton
          disabled={!!data.Error}
          color={data.PowerIsOn ? "error" : "success"}
          onClick={async () => { await setPowerState(data.Hostname, !data.PowerIsOn); } }
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
          { data && Object.values(data.hosts).map(hostRow) }
        </TableBody>
      </Table>
    </>
  )

}

export default App
