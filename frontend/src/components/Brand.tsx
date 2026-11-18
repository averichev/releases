import icon from '../assets/brand/icon.svg?no-inline';
import wordmark from '../assets/brand/wordmark.svg?no-inline';

export function Brand() {
  return <span className="brand-logo" role="img" aria-label="Релизы">
    <img className="brand-logo-icon" src={icon} alt="" width={128} height={128} />
    <img className="brand-logo-wordmark" src={wordmark} alt="" width={491} height={100} />
  </span>;
}
