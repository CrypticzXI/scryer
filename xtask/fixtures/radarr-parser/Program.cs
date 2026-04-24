using System.Text;
using System.Text.Json;
using NzbDrone.Core.Parser;

if (args.Length != 2)
{
    Console.Error.WriteLine("usage: RadarrParserRunner <input.jsonl> <output.jsonl>");
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
        var parsed = Parser.ParseMovieTitle(rawTitle);
        var row = new Dictionary<string, object?>
        {
            ["raw_title"] = rawTitle,
            ["parsed"] = parsed is null
                ? null
                : new Dictionary<string, object?>
                {
                    ["movie_titles"] = parsed.MovieTitles,
                    ["primary_movie_title"] = parsed.PrimaryMovieTitle,
                    ["movie_title"] = parsed.MovieTitle,
                    ["original_title"] = parsed.OriginalTitle,
                    ["release_title"] = parsed.ReleaseTitle,
                    ["simple_release_title"] = parsed.SimpleReleaseTitle,
                    ["year"] = parsed.Year == 0 ? null : parsed.Year,
                    ["edition"] = string.IsNullOrWhiteSpace(parsed.Edition) ? null : parsed.Edition,
                    ["release_group"] = parsed.ReleaseGroup,
                    ["release_hash"] = parsed.ReleaseHash,
                    ["imdb_id"] = parsed.ImdbId,
                    ["tmdb_id"] = parsed.TmdbId == 0 ? null : parsed.TmdbId,
                    ["hardcoded_subs"] = string.IsNullOrWhiteSpace(parsed.HardcodedSubs) ? null : parsed.HardcodedSubs,
                    ["quality_name"] = parsed.Quality?.Quality?.Name,
                    ["quality_source"] = parsed.Quality?.Quality?.Source.ToString(),
                    ["quality_resolution"] = parsed.Quality?.Quality?.Resolution,
                    ["quality_revision_version"] = parsed.Quality?.Revision?.Version,
                    ["quality_revision_real"] = parsed.Quality?.Revision?.Real,
                    ["quality_revision_is_repack"] = parsed.Quality?.Revision?.IsRepack,
                    ["languages"] = parsed.Languages.Select(language => language.Name).ToArray()
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
