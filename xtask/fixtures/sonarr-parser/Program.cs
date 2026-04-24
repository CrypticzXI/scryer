using System.Text;
using System.Text.Json;
using NzbDrone.Core.Parser;

if (args.Length != 2)
{
    Console.Error.WriteLine("usage: SonarrParserRunner <input.jsonl> <output.jsonl>");
    Environment.ExitCode = 2;
    return;
}

var inputPath = args[0];
var outputPath = args[1];
var jsonOptions = new JsonSerializerOptions
{
    WriteIndented = false
};

await using var output = new StreamWriter(outputPath, false, new UTF8Encoding(false));
using var input = new StreamReader(inputPath, Encoding.UTF8);

while (await input.ReadLineAsync() is { } line)
{
    if (string.IsNullOrWhiteSpace(line))
    {
        continue;
    }

    using var document = JsonDocument.Parse(line);
    var root = document.RootElement;
    var rawTitle = root.GetProperty("raw_title").GetString() ?? "";

    try
    {
        var parsed = Parser.ParseTitle(rawTitle);
        var row = new Dictionary<string, object?>
        {
            ["raw_title"] = rawTitle,
            ["parsed"] = parsed is null
                ? null
                : new Dictionary<string, object?>
                {
                    ["series_title"] = parsed.SeriesTitle,
                    ["series_title_without_year"] = parsed.SeriesTitleInfo?.TitleWithoutYear,
                    ["series_title_year"] = parsed.SeriesTitleInfo?.Year == 0 ? null : parsed.SeriesTitleInfo?.Year,
                    ["series_all_titles"] = parsed.SeriesTitleInfo?.AllTitles ?? Array.Empty<string>(),
                    ["release_title"] = parsed.ReleaseTitle,
                    ["release_group"] = parsed.ReleaseGroup,
                    ["release_hash"] = parsed.ReleaseHash,
                    ["season_number"] = parsed.SeasonNumber == 0 ? null : parsed.SeasonNumber,
                    ["episode_numbers"] = parsed.EpisodeNumbers,
                    ["absolute_episode_numbers"] = parsed.AbsoluteEpisodeNumbers,
                    ["special_absolute_episode_numbers"] = parsed.SpecialAbsoluteEpisodeNumbers,
                    ["air_date"] = string.IsNullOrWhiteSpace(parsed.AirDate) ? null : parsed.AirDate,
                    ["full_season"] = parsed.FullSeason,
                    ["is_partial_season"] = parsed.IsPartialSeason,
                    ["is_multi_season"] = parsed.IsMultiSeason,
                    ["is_season_extra"] = parsed.IsSeasonExtra,
                    ["is_split_episode"] = parsed.IsSplitEpisode,
                    ["is_mini_series"] = parsed.IsMiniSeries,
                    ["special"] = parsed.Special,
                    ["season_part"] = parsed.SeasonPart == 0 ? null : parsed.SeasonPart,
                    ["daily_part"] = parsed.DailyPart,
                    ["release_type"] = parsed.ReleaseType.ToString(),
                    ["quality_name"] = parsed.Quality?.Quality?.Name,
                    ["quality_source"] = parsed.Quality?.Quality?.Source.ToString(),
                    ["quality_resolution"] = parsed.Quality?.Quality?.Resolution,
                    ["quality_revision_version"] = parsed.Quality?.Revision?.Version,
                    ["quality_revision_real"] = parsed.Quality?.Revision?.Real,
                    ["quality_revision_is_repack"] = parsed.Quality?.Revision?.IsRepack,
                    ["languages"] = parsed.Languages.Select(language => language.Name).ToArray(),
                    ["release_tokens"] = parsed.ReleaseTokens
                },
            ["error"] = parsed is null ? "unparsed" : null
        };

        await output.WriteLineAsync(JsonSerializer.Serialize(row, jsonOptions));
    }
    catch (Exception exception)
    {
        var row = new Dictionary<string, object?>
        {
            ["raw_title"] = rawTitle,
            ["parsed"] = null,
            ["error"] = exception.Message
        };

        await output.WriteLineAsync(JsonSerializer.Serialize(row, jsonOptions));
    }
}
