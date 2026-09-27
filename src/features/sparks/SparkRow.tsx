import { Icon } from '../../components/Icon';
import type { SparkSummary } from '../../services/types';

interface SparkRowProps {
  spark: SparkSummary;
  copied: boolean;
  onCopy: (spark: SparkSummary) => void;
  onToggleFavorite: (spark: SparkSummary) => void;
}

/** One-line Spark: star (favorite toggle) + title + direct copy. The whole
 *  title area is the copy target so copying is always one click. */
export function SparkRow({ spark, copied, onCopy, onToggleFavorite }: SparkRowProps) {
  return (
    <li className={`spark-row${copied ? ' is-copied' : ''}`}>
      <button
        type="button"
        className={`row-star${spark.favorite ? '' : ' is-off'}`}
        aria-label={spark.favorite ? `Remove ${spark.title} from Favorites` : `Add ${spark.title} to Favorites`}
        aria-pressed={spark.favorite}
        title={spark.favorite ? 'Remove from Favorites' : 'Add to Favorites'}
        onClick={() => onToggleFavorite(spark)}
      >
        <Icon name={spark.favorite ? 'starFilled' : 'star'} size={17} strokeWidth={1.6} />
      </button>
      <button
        type="button"
        className="row-copy"
        onClick={() => onCopy(spark)}
        aria-label={`Copy ${spark.title}`}
        title={spark.summary || `Copy ${spark.title}`}
      >
        <span className="row-title">{spark.title}</span>
        <span className="row-copy-icon" aria-hidden="true">
          {copied ? (
            <>
              Copied <Icon name="check" size={16} strokeWidth={2} />
            </>
          ) : (
            <Icon name="copy" size={17} />
          )}
        </span>
      </button>
    </li>
  );
}
