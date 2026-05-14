import { createRoot } from 'react-dom/client';
import '@arco-design/web-react/dist/css/arco.css';
import './app.css';
import App from './App';

const root = document.getElementById('root');

if (!root) {
  throw new Error('Missing root element');
}

createRoot(root).render(<App />);
